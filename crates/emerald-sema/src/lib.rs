//! Name resolution + type checking over `emerald_parser::Program`
//! (plan-of-plans row 05, inception §17 steps 4–7 and §25.E).
//!
//! Plan 22's Decision log: every `Diagnostic` now carries a real
//! `span: (usize, usize)`, populated from whichever `Spanned<Expr>`/
//! `Spanned<Stmt>` node the check that raised it was actually
//! inspecting — the mismatched sub-expression for a type error, the
//! whole call for an arity error, the enclosing statement as the
//! honest fallback where no better candidate exists (`break`/`next`
//! outside a loop, etc.). `infer_expr_type`/`check_stmt`/etc. all
//! changed their *signatures* to take `&Spanned<Expr>`/`&Spanned<Stmt>`
//! rather than every match arm's binding pattern — see `ast.rs`'s own
//! `Spanned<T>` doc comment for why that's the cheaper edit.

use emerald_parser::{
  ActorDef, CaseArm, CasePattern, ClassDef, CompareOp, Contract, EnumDef, EnumVariant, Expr,
  Function, Item, ModuleDef, Param, Program, RescueClause, Spanned, Stmt, StringPart, TypeExpr,
  TypeParam,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
  Int64,
  Float64,
  String,
  Boolean,
  Void,
  /// `:foo` (plan 44's Decision log) — a genuinely separate type from
  /// `String`, not an alias: every distinct spelling appearing anywhere
  /// in a compilation unit is enumerable at parse time, so equality is
  /// a cheap interned-integer compare in codegen (`ValKind::Symbol`),
  /// not a string compare.
  Symbol,
  /// An instance of a user-defined class, named by its declaration.
  Class(String),
  /// A packed, contiguous array of a single element type
  /// (`spec/TYPE_SYSTEM.md` §8) — named `Array[Elem]` at the source level.
  Array(Box<Type>),
  /// A key/value container, named `Hash[K, V]` at the source level
  /// (plan 25's Decision log: `Int64` keys only, a flat linear-scan
  /// representation in codegen — not a real hash table).
  Hash(Box<Type>, Box<Type>),
  /// A closure's parameter types and return type. Unlike `Type::Class`,
  /// which is just a name backed by a separate `ClassInfo` registry,
  /// there's no such registry for lambdas — the signature has to travel
  /// with the type value itself (plan 10's Decision log). The
  /// *source-level* annotation is always the bare keyword `Proc`; the
  /// real signature here always comes from the `Expr::Lambda` a `Proc`
  /// local was bound to (see `check_stmt`'s `Let` case).
  Proc(Vec<Type>, Box<Type>),
  /// A not-yet-concrete type-parameter reference (plan 41's Decision
  /// log) — legal only inside the body of the generic function/class
  /// that declares it. Carries both the type parameter's own name
  /// (`"T"`, used purely for diagnostics — e.g. naming which parameter
  /// disagreed at a call site) and its bound interface's name
  /// (`"Comparable"`), so a method call on a `Generic`-typed receiver
  /// (`a.compare_to(b)`) can resolve the interface's required method
  /// directly from the receiver's own inferred type, without a
  /// separately threaded "which interface bounds the parameter
  /// currently in scope" context value. Plan 58's Decision log: the
  /// bound widens from a mandatory `String` to `Option<String>` — a
  /// generic CLASS's own type parameter can be genuinely unbounded
  /// (`class Box[T] ... end`), unlike a generic FUNCTION's
  /// (`GenericFunctionSig.bound` stays a plain `String`; a bound-less
  /// function type parameter is still rejected at registration time).
  /// A method call on a `bounds: []`-typed value is rejected outright
  /// — there is no interface to resolve the call against. Plan 88's
  /// Decision log: widened from a single `Option<String>` bound to a
  /// real bound SET (`[T: Bound1 + Bound2]`'s own conjunction) — a
  /// method call on a `Generic`-typed receiver now resolves against
  /// whichever bound in the set actually declares the method named,
  /// and a type argument must conform to EVERY bound in the set to be
  /// accepted at a generic call/instantiation site.
  Generic(String, Vec<String>),
  /// A fixed-arity anonymous tuple (plan 39's Decision log) — valid
  /// ONLY as a function's declared return type, never a parameter
  /// type, a field type, a `Let`'s local type, an array element type,
  /// or nested inside another tuple. Never constructed by the shared
  /// `resolve_type` (used for every param/field/`Let` annotation) —
  /// only `resolve_return_type` (`function_signature`'s own return-type
  /// resolution) ever produces this.
  Tuple(Vec<Type>),
  /// An instance of a user-declared `enum` (plan 52's Decision log) — a
  /// CLOSED set of variants, named by the enum's own declaration, the
  /// deliberate opposite of `Type::Class`'s open, extensible hierarchy.
  Enum(String),
  /// `Result[T, E]` (plan 53's Decision log) — a hardcoded, compiler-
  /// native compound type, generalizing plan 42's `Pair[K, V]`
  /// precedent. `Ok`/`Err` construction is checked only in the three
  /// expected-type-providing positions (`Let`/`Assign`/`Return`); `?`'s
  /// exact-`E`-match check compares this variant's own two `Type`s via
  /// ordinary structural `PartialEq` — no coercion, ever.
  Result(Box<Type>, Box<Type>),
  /// `Supervisor` (plan 57's Decision log) — the compile-time record of
  /// which actors a `supervise do ... end` block tracks, keyed by each
  /// spawn statement's own bound name (`None` for a bare, unnamed
  /// `.spawn` — Decision log: a real, disclosed narrowing, unreachable
  /// via `child(name)`, not a panic). Unlike `Type::Class`, which is
  /// just a name backed by a separate `ClassInfo` registry, there's no
  /// such registry for a `Supervisor` value — the *source-level*
  /// annotation is always the bare keyword `Supervisor`, and the real
  /// per-child class information travels with the value itself, the
  /// same "signature travels with the value" precedent `Type::Proc`
  /// already established (see `check_stmt`'s `Let` case).
  Supervisor(Vec<(Option<String>, Type)>),
  /// `Pair[K, V]` (plan 42's Decision log) — a hand-rolled, hard-coded
  /// compound type, the same way `Array[T]`/`Hash[K,V]` themselves
  /// already exist rather than a user-declarable generic class (plan
  /// 41's own contract covers generic *functions* and single-class
  /// `implements`, never generic *classes*). Never source-constructible
  /// directly — the only way to obtain one is `Hash[K,V].each`'s own
  /// block parameter, one per key/value slot, with exactly two
  /// accessors, `.key`/`.value`.
  Pair(Box<Type>, Box<Type>),
  /// A raw C string pointer (plan 59's Decision log) — a deliberately
  /// inert reference type: no `String` method (`.upcase`, `.length`,
  /// ...) dispatches on it at all. Never source-constructible directly;
  /// the only two ways to obtain or shed one are `s.to_cstring()`
  /// (`String -> CString`) and `String.from_cstring(ptr)`
  /// (`CString -> String?`), both real compiler-known intrinsics (plan
  /// 45's own dispatch mechanism), never an ordinary function/method.
  /// Nameable as an ordinary `Let`/field/param annotation too (`c:
  /// CString = ...`) — "deliberately inert" means no METHOD dispatches
  /// on it, not that it's unnameable; the real FFI trust boundary stays
  /// confined to `unsafe extern "C" { ... }` declarations and the two
  /// intrinsics above, which are the only ways to ever *produce* or
  /// *consume* one.
  CString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
  pub message: String,
  /// Plan 22's Decision log: real for every diagnostic raised while
  /// checking a `Spanned<Expr>`/`Spanned<Stmt>` — the span of whichever
  /// node the check was actually inspecting when it failed. `(0, 0)`
  /// only for the handful of module/class-registration-time
  /// diagnostics that predate any specific expression being in scope
  /// at all (see each call site's own comment).
  pub span: (usize, usize),
}

impl Diagnostic {
  fn new(message: impl Into<String>, span: (usize, usize)) -> Self {
    Self {
      message: message.into(),
      span,
    }
  }
}

#[derive(Debug, Clone)]
struct FunctionSig {
  params: Vec<Type>,
  return_type: Type,
  /// Plan 34's `&blk` marker, carried alongside the ordinary signature
  /// so a call site can tell whether it must attach a trailing block
  /// literal — mirrors `Function.block_param` (see its own doc comment
  /// for why this is a bare name, not a checkable `Type::Proc`).
  block_param: Option<String>,
  /// Plan 39's Decision log: `params[i]`'s declared name, parallel to
  /// `params`/`defaults` — lets a keyword-argument call site (`Expr::
  /// CallKw`) resolve each supplied name to its position, entirely at
  /// compile time.
  param_names: Vec<String>,
  /// Plan 39's Decision log: `params[i]`'s default value expression, if
  /// declared (`None` for every parameter without one — always `None`
  /// for every pre-plan-39 declaration). Filled into a call site's
  /// trailing omitted arguments, both positional and keyword.
  defaults: Vec<Option<Spanned<Expr>>>,
  /// Plan 39's Decision log: `Some(elem_ty)` when this function declares
  /// a trailing `*xs: Elem` splat parameter — every call-site argument
  /// beyond `params.len()` must have this type. `None` for every
  /// function that doesn't declare one.
  splat_elem: Option<Type>,
  /// Plan 62's `leaf-sema-static-provability`: carried alongside the
  /// ordinary signature so a call site's own static-provability check
  /// (`eval_const_bool`) has the callee's `requires` clauses in hand
  /// without needing a second lookup back into the raw `Program`.
  requires: Vec<Contract>,
  /// Plan 63's `leaf-sema-purity-check`: mirrors `Function.is_pure` so
  /// `check_purity`'s call-graph walk can look up whether a *callee*
  /// (by name, via `sigs`) claims purity without a second pass back
  /// into the raw `Program` — the same "carry it alongside the
  /// signature" precedent `requires` above already set for plan 62.
  is_pure: bool,
}

/// Shared by classes and modules (plan 12's Decision log — modules reuse
/// this registry, tagged `is_module: true`, rather than a separate
/// `modules` map threaded through every function that already carries
/// `classes`). A module's `fields` is always empty (`SEMANTICS.md`
/// §10.4 — no instance state to hold them).
#[derive(Debug, Clone)]
struct ClassInfo {
  /// Flattened — includes every ancestor's fields too (plan 32's
  /// Decision log), not just this class's own declaration.
  fields: HashMap<String, Type>,
  /// Flattened the same way — an inherited-but-not-overridden method
  /// resolves here to its defining ancestor's signature; an override
  /// replaces it.
  methods: HashMap<String, FunctionSig>,
  is_module: bool,
  /// `class Dog < Animal`'s `Animal` — `None` for a module (modules
  /// never inherit) or a class with no `<` clause. This is the class's
  /// OWN declared superclass, not a flattened chain.
  superclass: Option<String>,
  /// `implements Comparable` (plan 41's Decision log) — `None` for
  /// every class that doesn't declare one, always `None` for a module
  /// (the grammar's `ImplementsClause?` is only reachable from
  /// `ClassDef`). Plan 89's Decision log widens the tuple's second
  /// element to `Vec<TypeExpr>` — `implements Iterable[Int64]`'s
  /// `[Int64]`, binding the interface's own type parameter(s) to
  /// concrete types; empty for a bare `implements Comparable`.
  implements: Option<(String, Vec<TypeExpr>)>,
  /// Plan 52's Decision log: `Some(variants)` only for an `enum`
  /// registered into this SAME table — `[(variant_name, field_types)]`
  /// in declaration order. `fields`/`methods` stay empty and
  /// `is_module`/`superclass`/`implements` stay their defaults for an
  /// enum entry. A real, disclosed adaptation from the plan's own
  /// literal text (a separate `EnumInfo` registry): `classes` is
  /// already threaded through every function in this file that needs
  /// type resolution or `Expr::Call` dispatch — reusing it here (the
  /// same "modules share this table" precedent plan 12 already
  /// established) avoids a second new parameter cascading through
  /// dozens of already-large signatures for zero functional
  /// difference. `resolve_type` checks this field before falling
  /// through to its ordinary `Type::Class` branch.
  enum_variants: Option<Vec<(String, Vec<Type>)>>,
  /// Plan 54's Decision log: mirrors `is_module` exactly — an actor
  /// reuses this same `classes` registry (a real, disclosed adaptation:
  /// no separate `actors` map) tagged `is_actor: true`. `.new` rejects
  /// an actor receiver; `.spawn` requires one. `fields`/`methods` are
  /// populated exactly like an ordinary class's (an actor is flat, but
  /// still has real fields/methods, unlike a module); `superclass`/
  /// `implements`/`enum_variants` stay their defaults, since `ActorDef`
  /// has no grammar path to produce any of them.
  is_actor: bool,
  /// Plan 88's Decision log: a class method (or generic-class-template
  /// method) declaring its OWN `[U]`/`[U: Bound]` type parameter — one
  /// level of generics on top of whatever the class itself may already
  /// have — registers here instead of the ordinary `methods` table:
  /// its params/return type can't be resolved into one fixed
  /// `FunctionSig` at class-registration time, since `U` is only known
  /// at each call site. Reuses plan 41/58's existing top-level generic-
  /// function/class monomorphization STRATEGY (register a raw,
  /// unresolved signature; resolve/check it fresh at every call site
  /// that supplies a concrete type argument) rather than inventing a
  /// second, separate generics mechanism — see `GenericMethodSig`'s own
  /// doc comment for exactly how it differs from `GenericFunctionSig`.
  /// Empty for every class with no generic method at all.
  generic_methods: HashMap<String, GenericMethodSig>,
}

/// One required method inside an `interface ... end` body, kept as raw,
/// unresolved type-name strings (plan 41's Decision log) — `"Self"`
/// isn't resolvable via `resolve_type` until substituted with either a
/// concrete implementing class's name (conformance checking) or the
/// generic type parameter itself (generic-body checking), so resolving
/// eagerly at registration time would be premature. `type_params` is
/// this method's OWN `[U]`/`[U: Bound]` clause (plan 89's Decision log
/// — `fn map[U](f: Proc[T, U]): Array[U]`), empty for an ordinary
/// required method.
#[derive(Debug, Clone)]
struct InterfaceMethodInfo {
  method_name: String,
  type_params: Vec<TypeParam>,
  params_raw: Vec<(String, TypeExpr)>,
  return_type_raw: TypeExpr,
}

/// An `interface`'s own type-parameter clause plus its full required-
/// method set (plan 89's Decision log widens this from a single
/// non-generic required method to both a `type_params` clause and a
/// `Vec` of methods — `interface Iterable[T] ... end` with more than
/// one `fn` inside).
#[derive(Debug, Clone)]
struct InterfaceInfo {
  type_params: Vec<TypeParam>,
  methods: Vec<InterfaceMethodInfo>,
}

/// A top-level generic function's registration (plan 41's Decision
/// log) — kept separate from `sigs`/`FunctionSig` (never both at once
/// for the same name: the ordinary `sigs` pass skips every function
/// whose `type_params` is non-empty) since its raw parameter/return
/// type strings need per-call-site substitution before they can be
/// resolved into real `Type`s at all.
#[derive(Debug, Clone)]
struct GenericFunctionSig {
  type_param: String,
  bounds: Vec<String>,
  params_raw: Vec<(String, TypeExpr)>,
  return_type_raw: TypeExpr,
}

/// A class method's own `[U]`/`[U: Bound]` type parameter (plan 88's
/// Decision log) — `ClassInfo.generic_methods`'s own doc comment
/// explains why this exists as a registry separate from `methods`.
///
/// Deliberately NOT a reuse of `GenericFunctionSig` despite the
/// obvious structural overlap: `GenericFunctionSig.params_raw`/
/// `return_type_raw` only ever need a single, WHOLE-STRING equality
/// check against the type parameter's own bare name (`raw ==
/// &g.type_param`, `infer_expr_type`'s top-level-generic-function
/// `Expr::Call` arm) — a top-level generic function's own type
/// parameter can only ever appear as a BARE parameter type (`fn
/// identity[T: Bound](x: T): T`), never nested inside a compound shape.
/// A generic METHOD's own type parameter routinely appears nested
/// (`f: Proc[T, U]`, the plan's own worked example, where the class's
/// `T` is already concrete by the time this signature is built, and
/// only the method's own `U` remains free) — this needs real
/// structural unification against the argument's actual `Type`
/// (`infer_type_param_binding` below), not a whole-string compare.
/// `params_raw`/`return_type_raw` here are ALREADY class-level-
/// substituted (built via `substitute_type_params` against whichever
/// concrete/symbolic `T` the enclosing class instantiation supplies) —
/// only this struct's own `type_param` name remains genuinely free.
#[derive(Debug, Clone)]
struct GenericMethodSig {
  type_param: String,
  /// Empty for an unbounded method type parameter (plan 88's Decision
  /// log: unlike a top-level generic FUNCTION, which still requires at
  /// least one bound, a generic METHOD may be genuinely unbounded — the
  /// plan's own worked example, `map[U](f: Proc[T, U]): Array[U])`, has
  /// no bound on `U` at all).
  bounds: Vec<String>,
  params_raw: Vec<(String, TypeExpr)>,
  return_type_raw: TypeExpr,
}

/// Bundles the two registries a generic-aware type check needs beyond
/// `sigs`/`classes` — threaded as one additional parameter through the
/// whole `infer_expr_type`/`check_stmt` family (the same additive
/// pattern `self_fields: Option<&HashMap<...>>` already established),
/// always present (never `Option`) since it costs nothing to pass an
/// empty pair of maps through an ordinary, non-generic body.
struct GenericsCtx<'a> {
  interfaces: &'a HashMap<String, InterfaceInfo>,
  generic_sigs: &'a HashMap<String, GenericFunctionSig>,
}

/// Type names never reified as a real `Type::Class`/`Type::Enum`
/// instantiation-under-a-mangled-name — each has its own dedicated,
/// hardcoded `Type` variant instead (`resolve_type`'s own `Generic`
/// arm below). Shared with `mangle_type_expr` so both stay in sync.
const NATIVE_GENERIC_NAMES: [&str; 4] = ["Array", "Hash", "Pair", "Result"];

/// Resolves a `TypeExpr` — the parser's real, recursive type-expression
/// AST (plan 88's Decision log) — against `spec/TYPE_SYSTEM.md`'s
/// primitives, then against declared classes, consuming the AST's own
/// structure directly and recursively rather than parsing a formatted
/// string. Errors on any unrecognized/malformed shape rather than
/// silently accepting it.
fn resolve_type(
  texpr: &TypeExpr,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  match texpr {
    TypeExpr::Named(name) => resolve_named_type(name, classes),
    // Plan 88: `Array[Elem]`/`Hash[K, V]`/`Pair[K, V]`/`Result[T, E]`
    // are no longer their own hand-rolled grammar alternatives with
    // their own flat-string format — they're just `TypeExpr::Generic`
    // instances of the one general case, arity-checked here instead of
    // by the grammar. Each recurses on its own argument `TypeExpr`
    // directly, so `Array[Array[Int64]]`/`Hash[K, Array[V]]` etc. now
    // genuinely nest — the actual point of this plan's own AST change.
    TypeExpr::Generic(name, args) if name == "Array" => match args.as_slice() {
      [elem] => Ok(Type::Array(Box::new(resolve_type(elem, classes)?))),
      _ => Err(Diagnostic::new(
        format!(
          "`Array` takes exactly one type argument, found {}",
          args.len()
        ),
        (0, 0),
      )),
    },
    TypeExpr::Generic(name, args) if name == "Hash" => match args.as_slice() {
      [k, v] => Ok(Type::Hash(
        Box::new(resolve_type(k, classes)?),
        Box::new(resolve_type(v, classes)?),
      )),
      _ => Err(Diagnostic::new(
        format!(
          "`Hash` takes exactly two type arguments, found {}",
          args.len()
        ),
        (0, 0),
      )),
    },
    TypeExpr::Generic(name, args) if name == "Pair" => match args.as_slice() {
      [k, v] => Ok(Type::Pair(
        Box::new(resolve_type(k, classes)?),
        Box::new(resolve_type(v, classes)?),
      )),
      _ => Err(Diagnostic::new(
        format!(
          "`Pair` takes exactly two type arguments, found {}",
          args.len()
        ),
        (0, 0),
      )),
    },
    TypeExpr::Generic(name, args) if name == "Result" => match args.as_slice() {
      [t, e] => Ok(Type::Result(
        Box::new(resolve_type(t, classes)?),
        Box::new(resolve_type(e, classes)?),
      )),
      _ => Err(Diagnostic::new(
        format!(
          "`Result` takes exactly two type arguments, found {}",
          args.len()
        ),
        (0, 0),
      )),
    },
    // Plan 88: `Proc[Args..., Ret]` resolves DIRECTLY to a real,
    // written `Type::Proc` — no adjacent `Expr::Lambda` needed at all.
    // Every element is itself a full `TypeExpr`, so `Proc[Array[Int64],
    // Int64]` round-trips through this AST and resolves correctly.
    TypeExpr::Func(params, ret) => {
      let params = params
        .iter()
        .map(|p| resolve_type(p, classes))
        .collect::<Result<Vec<_>, _>>()?;
      let ret = resolve_type(ret, classes)?;
      Ok(Type::Proc(params, Box::new(ret)))
    }
    // Plan 58's Decision log: a generic-class instantiation (`Stack[
    // Int64]`) resolves to `Type::Class(mangled)` iff its mangled name
    // (`mangle_type_expr`) is already registered in `classes` — by the
    // time any caller reaches `resolve_type`, `check_program`'s own
    // collect+instantiate pass has already monomorphized every
    // instantiation actually written in the program, so a mangled name
    // absent here means the type argument itself didn't resolve (a
    // real, disclosed simplification: this path reports the SAME
    // "unknown type" diagnostic as the ordinary case below, rather than
    // re-deriving which specific type argument failed).
    // Plan 73's Decision log: a generic-ENUM instantiation (`Option[
    // Int64]`) mangles exactly the same way a generic-class one does —
    // the two share one mangled-name-keyed `classes` registry (plan
    // 52's own disclosed adaptation) — but must resolve to `Type::Enum`,
    // never `Type::Class`, the same split the ordinary (non-generic)
    // enum-vs-class check above already makes.
    TypeExpr::Generic(name, _) => {
      let mangled = mangle_type_expr(texpr);
      match classes.get(&mangled) {
        Some(info) if info.enum_variants.is_some() => Ok(Type::Enum(mangled)),
        Some(_) => Ok(Type::Class(mangled)),
        None => Err(Diagnostic::new(
          format!("unknown type `{name}[...]`"),
          (0, 0),
        )),
      }
    }
    // Legal ONLY as a function's declared return type
    // (`resolve_return_type`'s own dedicated handling below) — every
    // other annotation position (params, fields, `Let`) reaches
    // `resolve_type` directly, which has no meaning for a bare tuple
    // shape.
    TypeExpr::Tuple(_) => Err(Diagnostic::new(
      format!("tuple type `{texpr}` is only allowed as a function's declared return type"),
      (0, 0),
    )),
  }
}

/// `resolve_type`'s own `TypeExpr::Named` case — a plain identifier,
/// checked against primitives, then declared classes/modules/enums.
fn resolve_named_type(
  name: &str,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  match name {
    "Int64" => Ok(Type::Int64),
    "Float64" => Ok(Type::Float64),
    "String" => Ok(Type::String),
    "Void" => Ok(Type::Void),
    "Boolean" => Ok(Type::Boolean),
    "Symbol" => Ok(Type::Symbol),
    // Plan 59's Decision log: a `CString` value needs an ordinary
    // annotation position too (an intermediate `c: CString = s.
    // to_cstring()` local, an extern fn's own `CString`-typed
    // parameter) — "deliberately inert" (`Type::CString`'s own doc
    // comment) means no METHOD dispatches on it, not that it can't be
    // named.
    "CString" => Ok(Type::CString),
    // A bare `Proc` annotation carries no signature (see `Type::Proc`'s
    // doc comment) — reached only when `Proc` is written with no
    // bracket clause at all; the pre-existing lambda-literal-inference
    // path (`check_stmt`'s `Let` special case) overwrites this opaque
    // placeholder with the real signature recovered from the bound
    // `Expr::Lambda` before anything else ever observes it.
    "Proc" => Ok(Type::Proc(Vec::new(), Box::new(Type::Void))),
    // Plan 52's Decision log: checked before the ordinary `Type::Class`
    // branch below — an enum shares the same `classes` registry
    // (Decision log's disclosed adaptation) but must resolve to
    // `Type::Enum`, never `Type::Class`.
    other
      if classes
        .get(other)
        .is_some_and(|c| c.enum_variants.is_some()) =>
    {
      Ok(Type::Enum(other.to_string()))
    }
    // Modules are namespaces, not types (plan 12's Decision log) — a
    // module name is excluded here so `x: MathUtils = ...` correctly
    // falls through to the `unknown type` error below, not `Type::Class`.
    other if classes.get(other).is_some_and(|c| !c.is_module) => Ok(Type::Class(other.to_string())),
    other => Err(Diagnostic::new(format!("unknown type `{other}`"), (0, 0))),
  }
}

/// Plan 88: `"Stack[Int64]"` -> `"Stack$Int64"`, recursively (`"Stack[
/// Box[Int64]]"` -> `"Stack$Box$Int64"`) — the `TypeExpr`-native
/// replacement for the old string-splitting `mangle_type_name`, doubling
/// as both the synthesized `Type::Class` name AND codegen's `{ClassName}
/// _{method}` mangling prefix (Decision log), so nothing downstream
/// needs to know a mangled name came from a generic instantiation
/// rather than an ordinary source-declared class. `Array`/`Hash`/
/// `Pair`/`Result` are never mangled this way — each has its own
/// dedicated `Type` variant instead (`NATIVE_GENERIC_NAMES`) — so a
/// `TypeExpr` containing one renders via its own `Display` unchanged,
/// exactly matching the pre-plan-88 behavior this replaces.
fn mangle_type_expr(t: &TypeExpr) -> String {
  match t {
    TypeExpr::Generic(base, args) if !NATIVE_GENERIC_NAMES.contains(&base.as_str()) => {
      let mangled_args: Vec<String> = args.iter().map(mangle_type_expr).collect();
      format!("{base}${}", mangled_args.join("$"))
    }
    other => other.to_string(),
  }
}

/// Plan 88: whole-tree type-parameter substitution — a real, recursive
/// `TypeExpr`-to-`TypeExpr` rewrite replacing every `subst`-keyed
/// `Named` leaf, genuinely simpler than the byte-scanning identifier-run
/// substitution it replaces (`substitute_type_params`'s own pre-plan-88
/// implementation had to hand-roll word-boundary detection over a flat
/// string; walking a real tree needs none of that). Used by generic
/// CLASS instantiation (`resolve_substituted_type`) and by generic
/// METHOD registration (a method's own params/return, substituted
/// against the ENCLOSING class's type arguments before the method's own
/// free type parameter is registered into `GenericMethodSig`).
fn substitute_type_params(raw: &TypeExpr, subst: &HashMap<&str, &TypeExpr>) -> TypeExpr {
  match raw {
    TypeExpr::Named(name) => subst
      .get(name.as_str())
      .map(|t| (*t).clone())
      .unwrap_or_else(|| raw.clone()),
    TypeExpr::Generic(base, args) => TypeExpr::Generic(
      base.clone(),
      args
        .iter()
        .map(|a| substitute_type_params(a, subst))
        .collect(),
    ),
    TypeExpr::Tuple(parts) => TypeExpr::Tuple(
      parts
        .iter()
        .map(|p| substitute_type_params(p, subst))
        .collect(),
    ),
    TypeExpr::Func(params, ret) => TypeExpr::Func(
      params
        .iter()
        .map(|p| substitute_type_params(p, subst))
        .collect(),
      Box::new(substitute_type_params(ret, subst)),
    ),
  }
}

/// Plan 88: whole-tree scan — true if `name` appears as a standalone
/// `Named` leaf anywhere inside `ty`. The real-tree replacement for the
/// byte-scanning `type_references_any` this plan removes; used by
/// `check_generic_class_body` to recognize a field/param/return type
/// referencing the enclosing generic class's own type parameter.
fn type_references_any(ty: &TypeExpr, names: &HashSet<&str>) -> bool {
  match ty {
    TypeExpr::Named(name) => names.contains(name.as_str()),
    TypeExpr::Generic(_, args) => args.iter().any(|a| type_references_any(a, names)),
    TypeExpr::Tuple(parts) => parts.iter().any(|p| type_references_any(p, names)),
    TypeExpr::Func(params, ret) => {
      params.iter().any(|p| type_references_any(p, names)) || type_references_any(ret, names)
    }
  }
}

/// A resolved `Type`'s own `TypeExpr` re-encoding — the reverse
/// direction of `resolve_type`, needed only by `infer_type_param_binding`
/// /the generic-method call-site substitution it feeds: once a method's
/// own free type parameter (`U`) is bound to a concrete `Type` inferred
/// from an argument, that concrete `Type` needs to re-enter `substitute_
/// type_params`'s ordinary `TypeExpr`-to-`TypeExpr` substitution
/// machinery to resolve the REST of the signature (which may reference
/// `U` nested inside another compound shape, e.g. the plan's own worked
/// example's `Array[U]` return type). Best-effort for the handful of
/// `Type` shapes that can't round-trip through a source-level name at
/// all (`Type::Generic`/`Type::Supervisor`) — never actually reached in
/// practice, since neither is ever a legal generic-method type argument.
fn type_to_type_expr(t: &Type) -> TypeExpr {
  match t {
    Type::Int64 => TypeExpr::Named("Int64".to_string()),
    Type::Float64 => TypeExpr::Named("Float64".to_string()),
    Type::String => TypeExpr::Named("String".to_string()),
    Type::Boolean => TypeExpr::Named("Boolean".to_string()),
    Type::Void => TypeExpr::Named("Void".to_string()),
    Type::Symbol => TypeExpr::Named("Symbol".to_string()),
    Type::CString => TypeExpr::Named("CString".to_string()),
    Type::Class(name) | Type::Enum(name) | Type::Generic(name, _) => TypeExpr::Named(name.clone()),
    Type::Array(e) => TypeExpr::Generic("Array".to_string(), vec![type_to_type_expr(e)]),
    Type::Hash(k, v) => TypeExpr::Generic(
      "Hash".to_string(),
      vec![type_to_type_expr(k), type_to_type_expr(v)],
    ),
    Type::Pair(k, v) => TypeExpr::Generic(
      "Pair".to_string(),
      vec![type_to_type_expr(k), type_to_type_expr(v)],
    ),
    Type::Result(t, e) => TypeExpr::Generic(
      "Result".to_string(),
      vec![type_to_type_expr(t), type_to_type_expr(e)],
    ),
    Type::Proc(params, ret) => TypeExpr::Func(
      params.iter().map(type_to_type_expr).collect(),
      Box::new(type_to_type_expr(ret)),
    ),
    Type::Tuple(parts) => TypeExpr::Tuple(parts.iter().map(type_to_type_expr).collect()),
    Type::Supervisor(_) => TypeExpr::Named("Supervisor".to_string()),
  }
}

/// Plan 88: structural unification — finds what concrete `Type` a
/// generic method's own free type parameter (`param_name`, e.g. `U`)
/// would have to bind to for `raw` (the method's OWN, already class-
/// substituted declared type, e.g. `Proc[Int64, U]`) to match `actual`
/// (the real, inferred `Type` of the argument actually passed, e.g.
/// `Type::Proc([Type::Int64], Type::Int64)`). `None` when `param_name`
/// doesn't appear in `raw` at all, or when `raw`'s own shape doesn't
/// structurally match `actual` closely enough to say. Positional only
/// (first match wins within one call) — the caller
/// (`infer_expr_type`'s generic-method `MethodCall` dispatch) is
/// responsible for checking every parameter agrees on the SAME binding,
/// the same consistency check `infer_expr_type`'s existing top-level-
/// generic-function `Expr::Call` arm already performs for its own
/// (simpler, bare-identifier-only) case.
fn infer_type_param_binding(raw: &TypeExpr, actual: &Type, param_name: &str) -> Option<Type> {
  match raw {
    TypeExpr::Named(name) if name == param_name => Some(actual.clone()),
    TypeExpr::Named(_) => None,
    TypeExpr::Generic(base, args) => match (base.as_str(), actual) {
      ("Array", Type::Array(e)) => infer_type_param_binding(args.first()?, e, param_name),
      ("Hash", Type::Hash(k, v)) => infer_type_param_binding(args.first()?, k, param_name)
        .or_else(|| infer_type_param_binding(args.get(1)?, v, param_name)),
      ("Pair", Type::Pair(k, v)) => infer_type_param_binding(args.first()?, k, param_name)
        .or_else(|| infer_type_param_binding(args.get(1)?, v, param_name)),
      ("Result", Type::Result(t, e)) => infer_type_param_binding(args.first()?, t, param_name)
        .or_else(|| infer_type_param_binding(args.get(1)?, e, param_name)),
      _ => None,
    },
    TypeExpr::Tuple(parts) => match actual {
      Type::Tuple(ts) if ts.len() == parts.len() => parts
        .iter()
        .zip(ts)
        .find_map(|(p, t)| infer_type_param_binding(p, t, param_name)),
      _ => None,
    },
    TypeExpr::Func(params, ret) => match actual {
      Type::Proc(aparams, aret) if aparams.len() == params.len() => params
        .iter()
        .zip(aparams)
        .find_map(|(p, a)| infer_type_param_binding(p, a, param_name))
        .or_else(|| infer_type_param_binding(ret, aret, param_name)),
      _ => None,
    },
  }
}

/// Resolves `raw` (a generic method's own already class-substituted
/// declared type) to a concrete `Type`, with its own free type
/// parameter (`param_name`) bound to `concrete` — the call-site half of
/// `infer_type_param_binding`'s unification. Reuses the ordinary
/// `TypeExpr`-to-`TypeExpr` `substitute_type_params` plus `resolve_type`
/// rather than a bespoke `TypeExpr`-to-`Type` walk, so a `param_name`
/// nested arbitrarily deep (inside a user generic class's own type
/// argument, say) resolves exactly the same way any other substituted
/// type would.
fn resolve_with_type_param(
  raw: &TypeExpr,
  param_name: &str,
  concrete: &Type,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  let concrete_texpr = type_to_type_expr(concrete);
  let mut subst: HashMap<&str, &TypeExpr> = HashMap::new();
  subst.insert(param_name, &concrete_texpr);
  resolve_type(&substitute_type_params(raw, &subst), classes)
}

const GENERIC_INSTANTIATION_DEPTH_LIMIT: usize = 32;

/// Plan 58: resolves one field/param/return type string during a
/// generic class's own monomorphization — substitutes any type-
/// parameter name via `subst`, then, if the substituted result is
/// ITSELF a generic-class instantiation (a nested case like `Stack[Box[
/// Int64]]`'s `Box[Int64]`, or a self-referential `Node[T]`'s own
/// `next: Node[T]`), recursively instantiates it first so `resolve_type`
/// finds its mangled name already present in `classes`.
fn resolve_substituted_type(
  raw: &TypeExpr,
  subst: &HashMap<&str, &TypeExpr>,
  generic_classes: &HashMap<String, &ClassDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<Type, Diagnostic> {
  let substituted = substitute_type_params(raw, subst);
  if let TypeExpr::Generic(base, args) = &substituted {
    if generic_classes.contains_key(base.as_str()) {
      instantiate_generic_class(base, args, generic_classes, classes, in_progress)?;
    }
  }
  resolve_type(&substituted, classes)
}

/// Checks a single type argument against a single bound (one entry of a
/// type parameter's `bounds` set — plan 88's Decision log widened this
/// from a single optional bound to a real conjunctive set, so this is
/// now called once per bound, ALL of which must pass). Shared verbatim
/// between generic-class (`build_generic_class_info`) and generic-enum
/// (`build_generic_enum_info`) instantiation, which otherwise duplicated
/// this exact check.
fn check_generic_arg_bound(
  arg_class_name: &str,
  bound: &str,
  arg_display: &TypeExpr,
  type_param_name: &str,
  owner_kind: &str,
  owner_name: &str,
  classes: &HashMap<String, ClassInfo>,
) -> Result<(), Diagnostic> {
  let conforms = classes
    .get(arg_class_name)
    .is_some_and(|info| info.implements.as_ref().map(|(n, _)| n.as_str()) == Some(bound));
  if conforms {
    Ok(())
  } else {
    Err(Diagnostic::new(
      format!(
        "`{arg_display}` does not implement `{bound}`, required by generic {owner_kind} `{owner_name}`'s type parameter `{type_param_name}`"
      ),
      (0, 0),
    ))
  }
}

/// Resolves a generic instantiation's own type ARGUMENT to the mangled
/// class name it should be checked for interface conformance against —
/// monomorphizing it first if it's itself a nested user-generic-class
/// instantiation, falling back to a plain name/mangled string otherwise.
fn resolve_conformance_arg_name(
  arg: &TypeExpr,
  generic_classes: &HashMap<String, &ClassDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<String, Diagnostic> {
  if let TypeExpr::Generic(abase, aargs) = arg {
    if generic_classes.contains_key(abase.as_str()) {
      return instantiate_generic_class(abase, aargs, generic_classes, classes, in_progress);
    }
  }
  match arg {
    TypeExpr::Named(name) => Ok(name.clone()),
    other => Ok(mangle_type_expr(other)),
  }
}

/// Plan 58: builds the monomorphized `ClassInfo` for `base_name<
/// type_args>` — bound-checks each type argument, then substitutes
/// every type-parameter-named field/method param/return type via
/// `subst`. Doesn't push/pop `in_progress` itself (the caller,
/// `instantiate_generic_class`, owns that around this call) so an
/// early `?` return here never leaves a stale entry behind.
#[allow(clippy::too_many_arguments)]
fn build_generic_class_info(
  base_name: &str,
  c: &ClassDef,
  type_args: &[TypeExpr],
  subst: &HashMap<&str, &TypeExpr>,
  generic_classes: &HashMap<String, &ClassDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<ClassInfo, Diagnostic> {
  for (tp, arg) in c.type_params.iter().zip(type_args.iter()) {
    if tp.bounds.is_empty() {
      continue;
    }
    let arg_class_name = resolve_conformance_arg_name(arg, generic_classes, classes, in_progress)?;
    for bound in &tp.bounds {
      check_generic_arg_bound(
        &arg_class_name,
        bound,
        arg,
        &tp.name,
        "class",
        base_name,
        classes,
      )?;
    }
  }

  let mut fields = HashMap::new();
  for f in &c.fields {
    let ty = resolve_substituted_type(&f.ty, subst, generic_classes, classes, in_progress)?;
    fields.insert(f.name.clone(), ty);
  }

  let mut methods = HashMap::new();
  let mut generic_methods = HashMap::new();
  for m in &c.methods {
    // Plan 88's Decision log: a method's OWN type parameter (`map[U]`)
    // is a second, independent generic dimension on top of whatever
    // `c`'s own `type_params` already substituted here — registered
    // into `generic_methods` (resolved per call site, `ClassInfo`'s own
    // doc comment) instead of being pre-resolved into one fixed
    // `FunctionSig`, lifting the old blanket "generic methods are not
    // supported" rejection this branch used to raise unconditionally.
    if !m.type_params.is_empty() {
      let sig = generic_method_signature(base_name, m, subst, classes)?;
      generic_methods.insert(m.name.clone(), sig);
      continue;
    }
    let params = m
      .params
      .iter()
      .map(|p| resolve_substituted_type(&p.ty, subst, generic_classes, classes, in_progress))
      .collect::<Result<Vec<_>, _>>()?;
    let return_type =
      resolve_substituted_type(&m.return_type, subst, generic_classes, classes, in_progress)?;
    let param_names = m.params.iter().map(|p| p.name.clone()).collect();
    let defaults = m.params.iter().map(|p| p.default.clone()).collect();
    let splat_elem = m
      .splat_param
      .as_ref()
      .map(|p| resolve_substituted_type(&p.ty, subst, generic_classes, classes, in_progress))
      .transpose()?;
    methods.insert(
      m.name.clone(),
      FunctionSig {
        params,
        return_type,
        block_param: m.block_param.clone(),
        param_names,
        defaults,
        splat_elem,
        requires: m.requires.clone(),
        is_pure: m.is_pure,
      },
    );
  }

  Ok(ClassInfo {
    fields,
    methods,
    is_module: false,
    superclass: c.superclass.clone(),
    implements: c.implements.clone(),
    enum_variants: None,
    is_actor: false,
    generic_methods,
  })
}

/// Plan 88: registers one class method's own `[U]`/`[U: Bound]` type
/// parameter into a `GenericMethodSig` — shared by both `build_generic_
/// class_info` (a generic class TEMPLATE's own methods, `subst` maps
/// the class's own type parameters to their concrete instantiation
/// arguments) and `build_flattened_class_info` (an ordinary, non-
/// generic class, `subst` is empty). Exactly one type parameter is
/// required — a second is rejected here the same way a top-level
/// generic function's own registration already rejects more than one
/// (`GenericFunctionSig`'s own precedent) — but, unlike a top-level
/// function, a bound is NOT required (`GenericMethodSig.bounds`'s own
/// doc comment: the plan's own worked example, `map[U]`, is genuinely
/// unbounded).
fn generic_method_signature(
  class_name: &str,
  m: &Function,
  subst: &HashMap<&str, &TypeExpr>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<GenericMethodSig, Diagnostic> {
  if m.type_params.len() != 1 {
    return Err(Diagnostic::new(
      format!(
        "generic method `{class_name}#{}` declares {} type parameters — multiple type parameters are not supported",
        m.name,
        m.type_params.len()
      ),
      (0, 0),
    ));
  }
  let tp = &m.type_params[0];
  // A method's own free type parameter must never collide with a name
  // `subst` already binds (the enclosing class's own type parameter) —
  // otherwise `substitute_type_params` below would silently substitute
  // it away, leaving nothing left for call-site unification to bind.
  if subst.contains_key(tp.name.as_str()) {
    return Err(Diagnostic::new(
      format!(
        "generic method `{class_name}#{}`'s type parameter `{}` shadows the enclosing class's own type parameter of the same name",
        m.name, tp.name
      ),
      (0, 0),
    ));
  }
  let _ = classes; // reserved for a future bound-conformance pre-check; call sites re-check anyway.
  let params_raw = m
    .params
    .iter()
    .map(|p| (p.name.clone(), substitute_type_params(&p.ty, subst)))
    .collect();
  let return_type_raw = substitute_type_params(&m.return_type, subst);
  Ok(GenericMethodSig {
    type_param: tp.name.clone(),
    bounds: tp.bounds.clone(),
    params_raw,
    return_type_raw,
  })
}

/// Plan 58: monomorphizes `base_name<type_args>` into a real `ClassInfo`
/// inserted into `classes` under its mangled name (`mangle_type_name`).
/// Idempotent — a mangled name already present in `classes` returns
/// immediately (memoization: two `Stack[Int64]` instantiations anywhere
/// in the program only ever resolve this once).
///
/// Handles two distinct self-reference hazards via `in_progress`, the
/// stack of mangled names currently being resolved (Decision log): an
/// EXACT REPEAT of the name already at the top of `in_progress` is a
/// legitimate terminating self-reference (a linked-list-shaped `Node[T]`'s
/// own `next: Node[T]` field) — a placeholder `ClassInfo` is inserted
/// and this returns immediately without re-entering resolution (the
/// real one overwrites it once the enclosing call finishes building its
/// own fields). Anything else, once `in_progress.len()` reaches
/// `GENERIC_INSTANTIATION_DEPTH_LIMIT`, is a genuinely unbounded chain
/// (e.g. `Box[T]`'s own `wrapped: Box[Box[T]]` field, which mangles to
/// a strictly longer name at every step, never matching the immediately
/// enclosing frame) — rejected with a real diagnostic naming the class
/// and the depth bound. The same mechanism catches a cross-class two-
/// class cycle (`A[T]` has a `B[T]` field, `B[T]` has an `A[T]` field)
/// for free, with no separate graph-cycle algorithm: each step mangles
/// to the SAME name as an ancestor frame, not the immediately enclosing
/// one, so it's never treated as the legitimate self-reference case —
/// it just keeps growing `in_progress` until the depth bound rejects it.
fn instantiate_generic_class(
  base_name: &str,
  type_args: &[TypeExpr],
  generic_classes: &HashMap<String, &ClassDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<String, Diagnostic> {
  let mangled_args: Vec<String> = type_args.iter().map(mangle_type_expr).collect();
  let mangled = format!("{base_name}${}", mangled_args.join("$"));

  if classes.contains_key(&mangled) {
    return Ok(mangled);
  }
  if in_progress.last().map(String::as_str) == Some(mangled.as_str()) {
    classes.insert(mangled.clone(), empty_class_info(false));
    return Ok(mangled);
  }
  if in_progress.len() >= GENERIC_INSTANTIATION_DEPTH_LIMIT {
    return Err(Diagnostic::new(
      format!(
        "generic instantiation `{mangled}` exceeds the maximum nesting depth of {GENERIC_INSTANTIATION_DEPTH_LIMIT} — likely an unbounded recursive generic (a field whose type is a strictly larger instantiation of its own enclosing class, or a cycle between two generic classes)"
      ),
      (0, 0),
    ));
  }
  let Some(c) = generic_classes.get(base_name).copied() else {
    return Err(Diagnostic::new(
      format!("undefined generic class `{base_name}`"),
      (0, 0),
    ));
  };
  if c.type_params.len() != type_args.len() {
    return Err(Diagnostic::new(
      format!(
        "generic class `{base_name}` expects {} type argument(s), found {}",
        c.type_params.len(),
        type_args.len()
      ),
      (0, 0),
    ));
  }

  let subst: HashMap<&str, &TypeExpr> = c
    .type_params
    .iter()
    .map(|tp| tp.name.as_str())
    .zip(type_args.iter())
    .collect();

  in_progress.push(mangled.clone());
  let result = build_generic_class_info(
    base_name,
    c,
    type_args,
    &subst,
    generic_classes,
    classes,
    in_progress,
  );
  in_progress.pop();

  let info = result?;
  classes.insert(mangled.clone(), info);
  Ok(mangled)
}

/// A placeholder `ClassInfo` used only for `instantiate_generic_class`/
/// `instantiate_generic_enum`'s own self-reference hazard (their own
/// doc comments) — every field left at its empty/default value, since
/// the real one overwrites it once the enclosing call finishes.
fn empty_class_info(is_enum: bool) -> ClassInfo {
  ClassInfo {
    fields: HashMap::new(),
    methods: HashMap::new(),
    is_module: false,
    superclass: None,
    implements: None,
    enum_variants: if is_enum { Some(Vec::new()) } else { None },
    is_actor: false,
    generic_methods: HashMap::new(),
  }
}

/// Plan 73: `Option[T]`'s own monomorphization — the first GENERIC enum
/// this compiler ships, extending plan 41/58's exact generic-class
/// strategy (`instantiate_generic_class`/`build_generic_class_info`
/// immediately above) to `enum` declarations instead. Mirrors
/// `instantiate_generic_class` line for line: same mangled-name
/// memoization, same `in_progress` self-reference/depth-limit handling
/// (a recursive generic enum — `enum Tree[T] = Node(T, Tree[T]) | Leaf`
/// — hits the identical hazard a recursive generic class does). The one
/// real difference: builds a `ClassInfo` with `enum_variants: Some(...)`
/// populated instead of `fields`/`methods` — reusing plan 52's own
/// "enums share the classes table" adaptation, so `resolve_type`/
/// `check_case`/`find_all_variants` need no separate registry at all.
fn instantiate_generic_enum(
  base_name: &str,
  type_args: &[TypeExpr],
  generic_classes: &HashMap<String, &ClassDef>,
  generic_enums: &HashMap<String, &EnumDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<String, Diagnostic> {
  let mangled_args: Vec<String> = type_args.iter().map(mangle_type_expr).collect();
  let mangled = format!("{base_name}${}", mangled_args.join("$"));

  if classes.contains_key(&mangled) {
    return Ok(mangled);
  }
  if in_progress.last().map(String::as_str) == Some(mangled.as_str()) {
    classes.insert(mangled.clone(), empty_class_info(true));
    return Ok(mangled);
  }
  if in_progress.len() >= GENERIC_INSTANTIATION_DEPTH_LIMIT {
    return Err(Diagnostic::new(
      format!(
        "generic instantiation `{mangled}` exceeds the maximum nesting depth of {GENERIC_INSTANTIATION_DEPTH_LIMIT} — likely an unbounded recursive generic enum"
      ),
      (0, 0),
    ));
  }
  let Some(e) = generic_enums.get(base_name).copied() else {
    return Err(Diagnostic::new(
      format!("undefined generic enum `{base_name}`"),
      (0, 0),
    ));
  };
  if e.type_params.len() != type_args.len() {
    return Err(Diagnostic::new(
      format!(
        "generic enum `{base_name}` expects {} type argument(s), found {}",
        e.type_params.len(),
        type_args.len()
      ),
      (0, 0),
    ));
  }

  let subst: HashMap<&str, &TypeExpr> = e
    .type_params
    .iter()
    .map(|tp| tp.name.as_str())
    .zip(type_args.iter())
    .collect();

  in_progress.push(mangled.clone());
  let result = build_generic_enum_info(
    base_name,
    e,
    type_args,
    &subst,
    generic_classes,
    generic_enums,
    classes,
    in_progress,
  );
  in_progress.pop();

  let info = result?;
  classes.insert(mangled.clone(), info);
  Ok(mangled)
}

/// Plan 73: builds the monomorphized `ClassInfo` (its `enum_variants`
/// populated, `fields`/`methods` left empty) for `base_name<type_args>`
/// — bound-checks each type argument the same way `build_generic_class_
/// info` does, then substitutes every type-parameter-named variant field
/// type via `subst`. Doesn't push/pop `in_progress` itself (the caller,
/// `instantiate_generic_enum`, owns that), mirroring `build_generic_
/// class_info`'s own precedent exactly.
#[allow(clippy::too_many_arguments)]
fn build_generic_enum_info(
  base_name: &str,
  e: &EnumDef,
  type_args: &[TypeExpr],
  subst: &HashMap<&str, &TypeExpr>,
  generic_classes: &HashMap<String, &ClassDef>,
  generic_enums: &HashMap<String, &EnumDef>,
  classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>,
) -> Result<ClassInfo, Diagnostic> {
  for (tp, arg) in e.type_params.iter().zip(type_args.iter()) {
    if tp.bounds.is_empty() {
      continue;
    }
    let arg_class_name = if let TypeExpr::Generic(abase, aargs) = arg {
      if generic_classes.contains_key(abase.as_str()) {
        instantiate_generic_class(abase, aargs, generic_classes, classes, in_progress)?
      } else if generic_enums.contains_key(abase.as_str()) {
        instantiate_generic_enum(
          abase,
          aargs,
          generic_classes,
          generic_enums,
          classes,
          in_progress,
        )?
      } else {
        mangle_type_expr(arg)
      }
    } else if let TypeExpr::Named(name) = arg {
      name.clone()
    } else {
      mangle_type_expr(arg)
    };
    for bound in &tp.bounds {
      check_generic_arg_bound(
        &arg_class_name,
        bound,
        arg,
        &tp.name,
        "enum",
        base_name,
        classes,
      )?;
    }
  }

  let mut variants = Vec::new();
  for v in &e.variants {
    let mut field_types = Vec::new();
    for f in &v.fields {
      let substituted = substitute_type_params(f, subst);
      if let TypeExpr::Generic(base, args) = &substituted {
        if generic_classes.contains_key(base.as_str()) {
          instantiate_generic_class(base, args, generic_classes, classes, in_progress)?;
        } else if generic_enums.contains_key(base.as_str()) {
          instantiate_generic_enum(
            base,
            args,
            generic_classes,
            generic_enums,
            classes,
            in_progress,
          )?;
        }
      }
      field_types.push(resolve_type(&substituted, classes)?);
    }
    variants.push((v.name.clone(), field_types));
  }

  Ok(ClassInfo {
    fields: HashMap::new(),
    methods: HashMap::new(),
    is_module: false,
    superclass: None,
    implements: None,
    enum_variants: Some(variants),
    is_actor: false,
    generic_methods: HashMap::new(),
  })
}

/// Plan 58: whole-program walk collecting every generic-class-
/// instantiation type-name string actually written anywhere (`Let`
/// annotations, ordinary function/method/actor param/return types,
/// class/actor field types) — mirrors the established recursive-
/// `Stmt`-tree-walking pattern (plan 57's `collect_supervised_classes_
/// in_stmt`) covering If/While/For/ForRange/Begin/Case/MatchResult
/// nested bodies. A generic class TEMPLATE's own raw field/method types
/// are deliberately skipped — those are handled via substitution inside
/// `build_generic_class_info` instead, not collected here.
fn collect_generic_instantiation_typenames(program: &Program) -> Vec<TypeExpr> {
  let mut out = Vec::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        if f.type_params.is_empty() {
          for p in &f.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&f.return_type, &mut out);
          if let Some(p) = &f.splat_param {
            push_generic_typename(&p.ty, &mut out);
          }
        }
        for s in &f.body {
          collect_typenames_in_stmt(s, &mut out);
        }
      }
      Item::Class(c) => {
        if c.type_params.is_empty() {
          for field in &c.fields {
            push_generic_typename(&field.ty, &mut out);
          }
          for m in &c.methods {
            for p in &m.params {
              push_generic_typename(&p.ty, &mut out);
            }
            push_generic_typename(&m.return_type, &mut out);
            for s in &m.body {
              collect_typenames_in_stmt(s, &mut out);
            }
          }
        }
      }
      Item::Module(md) => {
        for m in &md.methods {
          for p in &m.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&m.return_type, &mut out);
          for s in &m.body {
            collect_typenames_in_stmt(s, &mut out);
          }
        }
      }
      Item::Actor(a) => {
        for field in &a.fields {
          push_generic_typename(&field.ty, &mut out);
        }
        for m in &a.methods {
          for p in &m.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&m.return_type, &mut out);
          for s in &m.body {
            collect_typenames_in_stmt(s, &mut out);
          }
        }
      }
      Item::Stmt(s) => collect_typenames_in_stmt(s, &mut out),
      Item::Test { body, .. } => {
        for s in body {
          collect_typenames_in_stmt(s, &mut out);
        }
      }
      Item::Enum(_) | Item::Interface(_) | Item::Require(_) | Item::Error | Item::Extern(_) => {}
    }
  }
  out
}

fn push_generic_typename(ty: &TypeExpr, out: &mut Vec<TypeExpr>) {
  if let TypeExpr::Generic(base, _) = ty {
    if !NATIVE_GENERIC_NAMES.contains(&base.as_str()) {
      out.push(ty.clone());
    }
  }
}

fn collect_typenames_in_stmt(stmt: &Spanned<Stmt>, out: &mut Vec<TypeExpr>) {
  match &stmt.node {
    Stmt::Let { ty, .. } => push_generic_typename(ty, out),
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      for s in then_branch {
        collect_typenames_in_stmt(s, out);
      }
      if let Some(eb) = else_branch {
        for s in eb {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
      for s in body {
        collect_typenames_in_stmt(s, out);
      }
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_typenames_in_stmt(s, out);
      }
      for r in rescues {
        for s in &r.body {
          collect_typenames_in_stmt(s, out);
        }
      }
      if let Some(e) = ensure {
        for s in e {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        for s in body {
          collect_typenames_in_stmt(s, out);
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::MatchResult {
      ok_body, err_body, ..
    } => {
      for s in ok_body {
        collect_typenames_in_stmt(s, out);
      }
      for s in err_body {
        collect_typenames_in_stmt(s, out);
      }
    }
    Stmt::SetField { .. }
    | Stmt::SetIndex { .. }
    | Stmt::Assign { .. }
    | Stmt::MultiAssign { .. }
    | Stmt::Return(_)
    | Stmt::Break
    | Stmt::Next
    | Stmt::Expr(_)
    | Stmt::Raise(_)
    | Stmt::Yield(_)
    | Stmt::Retry => {}
  }
}

/// Plan 58: checks a generic class TEMPLATE's method bodies exactly
/// once, against `Type::Generic`-substituted field/param/return types
/// (mirroring `check_generic_function_body`'s own precedent) — never
/// once per instantiation. Doesn't reuse `check_method_body` directly:
/// that function's own `resolve_type` calls can't resolve a bare type-
/// parameter name like `"T"` at all, since `resolve_type` only ever
/// knows about primitives and registered classes.
fn check_generic_class_body(
  c: &ClassDef,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let no_method_type_params: Vec<TypeParam> = Vec::new();
  let resolve_maybe_generic =
    |ty: &TypeExpr, method_type_params: &[TypeParam]| -> Result<Type, Diagnostic> {
      resolve_maybe_generic_type(ty, &c.type_params, method_type_params, classes)
    };

  let mut self_fields: HashMap<String, Type> = HashMap::new();
  for field in &c.fields {
    self_fields.insert(
      field.name.clone(),
      resolve_maybe_generic(&field.ty, &no_method_type_params)?,
    );
  }

  for m in &c.methods {
    // Plan 88's Decision log: a method's OWN `[U]`/`[U: Bound]` type
    // parameter is a real, disclosed gap here — its BODY is never
    // template-checked against the symbolic `U` (mirroring this
    // function's own pre-existing "opaque `Type::Class` placeholder"
    // simplification for a compound self-referencing field/param/
    // return type, immediately above); only call-site argument/return-
    // type checking is fully implemented (`infer_expr_type`'s
    // `generic_methods` dispatch). Skipped here entirely rather than
    // risk a confusing, spurious "unknown type `U`" diagnostic from a
    // body-check this compiler can't yet perform meaningfully.
    if !m.type_params.is_empty() {
      continue;
    }
    let mut env = HashMap::new();
    // Plan 72's Decision log: a generic class template's own method
    // parameters are immutable by construction, the same as an
    // ordinary class's (`check_method_body`) — no `var` slot exists on
    // a parameter regardless of which class/method-checking path binds
    // it.
    let mut mutable_locals: HashSet<String> = HashSet::new();
    for p in &m.params {
      env.insert(
        p.name.clone(),
        resolve_maybe_generic(&p.ty, &no_method_type_params)?,
      );
    }
    let declared_return = resolve_maybe_generic(&m.return_type, &no_method_type_params)?;
    check_block(
      &m.body,
      &mut env,
      &mut mutable_locals,
      sigs,
      classes,
      Some(&self_fields),
      &declared_return,
      false,
      false,
      m.block_param.is_some(),
      gctx,
    )?;
    check_implicit_return(
      &m.body,
      &env,
      sigs,
      classes,
      Some(&self_fields),
      &declared_return,
      &format!("{}#{}", c.name, m.name),
      gctx,
    )?;
  }
  Ok(())
}

/// Plan 52's Decision log: a flat, single-namespace variant-owner
/// lookup — the same "which registry does this bare name belong to"
/// cascade `resolve_type` already uses for primitives / classes /
/// `Array[...]` / `Hash[...]`, applied to `Expr::Call`'s construction-
/// site check instead of a type annotation. Iterates every enum
/// currently registered in `classes` (an entry's `enum_variants` is
/// `Some` only for an actual enum, per the Decision log's disclosed
/// "enums share the classes table" adaptation) for a variant named
/// `name`.
///
/// Plan 73's Decision log: registration-time collision checks still
/// guarantee at most one ORDINARY (non-generic) enum ever owns a given
/// variant name, but a generic enum monomorphized more than once (e.g.
/// both `Option[Int64]` and `Option[String]` used in the same program)
/// genuinely has *multiple* enums — `Option$Int64`, `Option$String` —
/// both declaring `Some`/`None`. Returns every match rather than just
/// the first (`HashMap` iteration order is otherwise undefined) so the
/// caller can disambiguate.
fn find_all_variants(name: &str, classes: &HashMap<String, ClassInfo>) -> Vec<(String, Vec<Type>)> {
  let mut out = Vec::new();
  for (enum_name, info) in classes {
    if let Some(variants) = &info.enum_variants {
      if let Some((_, field_types)) = variants.iter().find(|(vn, _)| vn == name) {
        out.push((enum_name.clone(), field_types.clone()));
      }
    }
  }
  // Deterministic order (`classes` is a `HashMap`) — cosmetic only (it
  // never changes which candidate is chosen), but keeps a "candidates:
  // ..." diagnostic's own wording reproducible across compiler runs.
  out.sort_by(|a, b| a.0.cmp(&b.0));
  out
}

/// Plan 73's Decision log: `Option[T]`'s own compiler-synthesized enum
/// name is always `"Option"` (bare, unresolved template) or `"Option$
/// ..."` (monomorphized, per `mangle_type_name`'s `$`-separator
/// convention) — used wherever a diagnostic or a dispatch decision needs
/// to recognize "this is specifically an `Option`," not just "some
/// enum" (e.g. the direct-`.method`-on-`Option` diagnostic below, and
/// `?.`/`??`'s own typing rules).
fn is_option_type(ty: &Type, _classes: &HashMap<String, ClassInfo>) -> bool {
  matches!(ty, Type::Enum(name) if name == "Option" || name.starts_with("Option$"))
}

/// Plan 73's Decision log: resolves and checks a variant CONSTRUCTION
/// (`Expr::Call(name, args)`, or a bare zero-arg `Expr::Ident(name)` —
/// `Option[T]`'s own `None`) directly against one SPECIFIC, already-known
/// enum — used at every "expected-type-providing position" this
/// compiler already has (`Let`'s declared type, `Assign`'s recorded
/// type, `Return`'s threaded return type, and a function body's own
/// final implicit-return expression), the same precedent `Result[T,
/// E]`'s `check_result_construction` established for `Ok`/`Err`, gener-
/// alized to any enum (this is what makes a bare, unqualified `None`
/// resolvable at all — seeing which SPECIFIC `Option[T]` instantiation
/// it means is otherwise ambiguous the moment two coexist, per
/// `find_all_variants`'s own doc comment). Returns `Ok(true)` if `expr`
/// really was shaped like one of `enum_name`'s own variants (already
/// fully checked — the caller should treat this position as handled);
/// `Ok(false)` if `expr` wasn't a construction of ANY variant of this
/// specific enum — the caller falls back to its own ordinary
/// `infer_expr_type`-based check (which still gives the right diagnostic
/// when `expr` is, say, an ordinary function call rather than a variant
/// construction at all).
#[allow(clippy::too_many_arguments)]
fn check_enum_variant_construction(
  enum_name: &str,
  expr: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<bool, Diagnostic> {
  let (name, args): (&str, &[Spanned<Expr>]) = match &expr.node {
    Expr::Call(n, a) => (n.as_str(), a.as_slice()),
    Expr::Ident(n) => (n.as_str(), &[]),
    _ => return Ok(false),
  };
  let Some(info) = classes.get(enum_name) else {
    return Ok(false);
  };
  let Some(variants) = &info.enum_variants else {
    return Ok(false);
  };
  let Some((_, field_types)) = variants.iter().find(|(vn, _)| vn == name) else {
    return Ok(false);
  };
  check_args(
    name,
    args,
    field_types,
    env,
    sigs,
    classes,
    self_fields,
    gctx,
  )?;
  Ok(true)
}

/// Shared by `check_generic_class_body`'s own field/param/return-type
/// resolution — a type appearing directly as a bare `class_type_params`/
/// `method_type_params` name resolves to `Type::Generic`; anything else
/// falls through to the ordinary `resolve_type`, with a COMPOUND type
/// referencing either type-parameter set (`Node[T]`'s own `succ: Node[
/// T]` field) treated as an opaque, not-yet-instantiated `Type::Class`
/// placeholder (this function's own pre-existing "real, disclosed
/// simplification" — see `check_generic_class_body`'s own doc comment
/// for the full rationale, unchanged by plan 88 beyond widening `bound`
/// to `bounds` and accepting a second, method-level type-parameter set).
fn resolve_maybe_generic_type(
  ty: &TypeExpr,
  class_type_params: &[TypeParam],
  method_type_params: &[TypeParam],
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  if let TypeExpr::Named(name) = ty {
    if let Some(tp) = method_type_params
      .iter()
      .chain(class_type_params.iter())
      .find(|tp| &tp.name == name)
    {
      return Ok(Type::Generic(tp.name.clone(), tp.bounds.clone()));
    }
  }
  match resolve_type(ty, classes) {
    Ok(t) => Ok(t),
    Err(e) => {
      let names: HashSet<&str> = class_type_params
        .iter()
        .chain(method_type_params.iter())
        .map(|tp| tp.name.as_str())
        .collect();
      if type_references_any(ty, &names) {
        Ok(Type::Class(ty.to_string()))
      } else {
        Err(e)
      }
    }
  }
}

/// Plan 73's Decision log: the inverse of `resolve_type` for the finite
/// slice of `Type` values `?.`'s own result-rewrapping needs to name as
/// an `Option[T]` type ARGUMENT string (fed into `mangle_type_name` to
/// look up an already-instantiated `"Option${arg}"` in `classes`).
/// `None` for anything `Option[T]` was never going to be instantiated
/// over anyway (`Tuple`/`Proc`/`Supervisor`/... — none of these are
/// legal generic-instantiation arguments in this compiler at all).
fn type_annotation_string(ty: &Type) -> Option<String> {
  match ty {
    Type::Int64 => Some("Int64".to_string()),
    Type::Float64 => Some("Float64".to_string()),
    Type::String => Some("String".to_string()),
    Type::Boolean => Some("Boolean".to_string()),
    Type::Void => Some("Void".to_string()),
    Type::Symbol => Some("Symbol".to_string()),
    Type::CString => Some("CString".to_string()),
    Type::Class(name) | Type::Enum(name) => Some(name.clone()),
    Type::Array(elem) => type_annotation_string(elem).map(|e| format!("Array[{e}]")),
    Type::Hash(k, v) => {
      let k = type_annotation_string(k)?;
      let v = type_annotation_string(v)?;
      Some(format!("Hash[{k}, {v}]"))
    }
    Type::Generic(_, _)
    | Type::Proc(_, _)
    | Type::Tuple(_)
    | Type::Result(_, _)
    | Type::Supervisor(_)
    | Type::Pair(_, _) => None,
  }
}

/// Resolves a function's declared return-type annotation only — the one
/// place a `TypeExpr::Tuple` annotation gains real meaning (plan 39's
/// Decision log). Every other annotation site (params, fields, `Let`)
/// keeps calling the shared `resolve_type` directly, which rejects a
/// bare tuple shape outright — the same "grammar permits it everywhere,
/// only one specific resolution path gives it real meaning" precedent
/// `resolve_type`'s own bare `"Proc"` case already established.
fn resolve_return_type(
  texpr: &TypeExpr,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  if let TypeExpr::Tuple(parts) = texpr {
    let elem_types = parts
      .iter()
      .map(|part| resolve_type(part, classes))
      .collect::<Result<Vec<_>, _>>()?;
    return Ok(Type::Tuple(elem_types));
  }
  resolve_type(texpr, classes)
}

/// Plan 59's Decision log: an `unsafe extern "C" { ... }` fn's own
/// param/return type allow-list — `{Int64, Float64, String, CString}`
/// for a parameter, plus `Void` for a return type. Narrower than the
/// real `Type` enum's own surface on purpose (no `Class`/`Array`/
/// `Hash`/`Symbol`/`Tuple`/`Boolean`/...): a C ABI has no idea what any
/// of those Emerald-internal representations mean, and — per the
/// Decision log's own verified finding — the real `Type` enum has no
/// fixed-width integer narrower than `Int64` to marshal into at all.
/// `is_return_position` gates `Void`, legal only as a return type,
/// never a parameter type (mirroring `ret_kind_for_type`/
/// `value_kind_for_type`'s own return-vs-param asymmetry elsewhere in
/// this codebase).
fn resolve_extern_type(texpr: &TypeExpr, is_return_position: bool) -> Result<Type, Diagnostic> {
  let Some(name) = texpr.as_named() else {
    return Err(Diagnostic::new(
      format!(
        "`{texpr}` is not a supported extern \"C\" type — only Int64, Float64, String, CString{} are marshalable across the FFI boundary",
        if is_return_position { ", and Void (return only)" } else { "" }
      ),
      (0, 0),
    ));
  };
  match name {
    "Int64" => Ok(Type::Int64),
    "Float64" => Ok(Type::Float64),
    "String" => Ok(Type::String),
    "CString" => Ok(Type::CString),
    "Void" if is_return_position => Ok(Type::Void),
    other => Err(Diagnostic::new(
      format!(
        "`{other}` is not a supported extern \"C\" type — only Int64, Float64, String, CString{} are marshalable across the FFI boundary",
        if is_return_position {
          ", and Void (return only)"
        } else {
          ""
        }
      ),
      (0, 0),
    )),
  }
}

/// Plan 43's Decision log originally widened this beyond plain equality
/// for `T?`'s own `nil → T?`/`T → T?` widening rules; plan 73 removes
/// `T?`/`Nil` from the type system entirely, so this is exact structural
/// equality again — `Option[T]` carries no implicit widening of its own
/// (a bare `T` is never assignable to a declared `Option[T]`; the source
/// must write `Some(value)` explicitly, checked separately by
/// `check_enum_variant_construction`).
fn is_assignable(actual: &Type, declared: &Type) -> bool {
  actual == declared
}

fn function_signature(
  f: &Function,
  classes: &HashMap<String, ClassInfo>,
) -> Result<FunctionSig, Diagnostic> {
  // Plan 41's Decision log: the only callers that can ever reach this
  // with a `type_params`-non-empty `Function` are `build_flattened_
  // class_info` (a class method) and `module_info` (a module method) —
  // `check_program`'s own `sigs` registration pass skips a top-level
  // generic function entirely, routing it through `generic_sigs`
  // instead. Caught here, before `resolve_type` ever sees a bare `"T"`
  // and fails with a confusing "unknown type" diagnostic instead.
  if !f.type_params.is_empty() {
    return Err(Diagnostic::new(
      format!("generic methods are not supported yet (`{}`)", f.name),
      (0, 0),
    ));
  }
  let params = f
    .params
    .iter()
    .map(|p| resolve_type(&p.ty, classes))
    .collect::<Result<Vec<_>, _>>()?;
  let return_type = resolve_return_type(&f.return_type, classes)?;
  let param_names = f.params.iter().map(|p| p.name.clone()).collect();
  let defaults = f.params.iter().map(|p| p.default.clone()).collect();
  let splat_elem = f
    .splat_param
    .as_ref()
    .map(|p| resolve_type(&p.ty, classes))
    .transpose()?;
  Ok(FunctionSig {
    params,
    return_type,
    block_param: f.block_param.clone(),
    param_names,
    defaults,
    splat_elem,
    requires: f.requires.clone(),
    is_pure: f.is_pure,
  })
}

/// A substituting variant of `function_signature` — identical except
/// each param/return/splat type is first rewritten via `substitute_
/// type_params` against `subst` before `resolve_type`/`resolve_return_
/// type` ever sees it. An empty `subst` makes this behaviorally
/// identical to `function_signature` (substituting against an empty map
/// is a no-op rewrite — `substitute_type_params`'s own base case),
/// so every existing non-generic-interface call site keeps its old
/// behavior unchanged. A non-empty `subst` is plan 89's own new case: a
/// class method whose declared type references its enclosing class's
/// `implements Iterable[Int64]`-bound interface type parameter (`T`)
/// directly, with no `[U]` of its own (`build_flattened_class_info`'s
/// own `interface_type_param_subst` call is what builds `subst` here).
fn function_signature_with_subst(
  f: &Function,
  subst: &HashMap<&str, &TypeExpr>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<FunctionSig, Diagnostic> {
  if !f.type_params.is_empty() {
    return Err(Diagnostic::new(
      format!("generic methods are not supported yet (`{}`)", f.name),
      (0, 0),
    ));
  }
  let params = f
    .params
    .iter()
    .map(|p| resolve_type(&substitute_type_params(&p.ty, subst), classes))
    .collect::<Result<Vec<_>, _>>()?;
  let return_type = resolve_return_type(&substitute_type_params(&f.return_type, subst), classes)?;
  let param_names = f.params.iter().map(|p| p.name.clone()).collect();
  let defaults = f.params.iter().map(|p| p.default.clone()).collect();
  let splat_elem = f
    .splat_param
    .as_ref()
    .map(|p| resolve_type(&substitute_type_params(&p.ty, subst), classes))
    .transpose()?;
  Ok(FunctionSig {
    params,
    return_type,
    block_param: f.block_param.clone(),
    param_names,
    defaults,
    splat_elem,
    requires: f.requires.clone(),
    is_pure: f.is_pure,
  })
}

/// Walks `name`'s `superclass` chain via `classes`'s already-registered
/// `superclass` links (plan 32's Decision log) — this only needs pass
/// 1's stub registration (names + `superclass`, not yet flattened
/// fields/methods) since it just follows the chain of names. Returns
/// root-to-leaf order (the ultimate ancestor first, `name` itself
/// last). Errors by name on a cycle (a visited-set catches it the
/// moment any name reappears while walking up) or an undefined/module
/// ancestor name, rather than looping or panicking.
fn resolve_chain(
  name: &str,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Vec<String>, Diagnostic> {
  let mut chain = Vec::new();
  let mut visited = HashSet::new();
  let mut current = name.to_string();
  loop {
    if !visited.insert(current.clone()) {
      return Err(Diagnostic::new(
        format!("cyclic inheritance detected involving class `{current}`"),
        (0, 0),
      ));
    }
    let info = classes
      .get(&current)
      .ok_or_else(|| Diagnostic::new(format!("undefined class `{current}`"), (0, 0)))?;
    if info.is_module {
      return Err(Diagnostic::new(
        format!(
          "cannot inherit from module `{current}` — modules are namespaces, not instantiable"
        ),
        (0, 0),
      ));
    }
    chain.push(current.clone());
    match &info.superclass {
      Some(parent) => current = parent.clone(),
      None => break,
    }
  }
  chain.reverse();
  Ok(chain)
}

/// Plan 89: builds the substitution map from a generic interface's own
/// type parameter name(s) to the concrete `TypeExpr`(s) `c`'s own
/// `implements Interface[Args]` clause binds them to — empty if `c`
/// declares no `implements` clause, names a non-generic interface (bare
/// `implements Comparable`), or names an interface that isn't
/// registered at all. Arity mismatches and an unknown interface name are
/// left entirely to `check_interface_conformance`'s own diagnostics —
/// this helper degrades to an empty substitution rather than erroring,
/// so a bad `implements` clause is reported once, clearly, there,
/// rather than risking a second, differently-worded failure here.
fn interface_type_param_subst<'a>(
  c: &'a ClassDef,
  interfaces: &'a HashMap<String, InterfaceInfo>,
) -> HashMap<&'a str, &'a TypeExpr> {
  let mut subst = HashMap::new();
  if let Some((iface_name, args)) = &c.implements {
    if let Some(iface) = interfaces.get(iface_name.as_str()) {
      for (tp, arg) in iface.type_params.iter().zip(args.iter()) {
        subst.insert(tp.name.as_str(), arg);
      }
    }
  }
  subst
}

/// Builds one class's FLATTENED field/method tables (plan 32's
/// Decision log) — walks `name`'s chain root-to-leaf via `class_defs`
/// (the raw `ClassDef`s, keyed by name; independent of processing
/// order, since every lookup here goes straight to the source
/// declaration rather than a previously-computed `ClassInfo`), merging
/// each ancestor's own fields (erroring on a name already declared by
/// an earlier ancestor) and methods (an existing same-named entry is a
/// real override, checked for an exact signature match before being
/// replaced) in turn. `classes` (pass 1's stub registration) is only
/// used for name resolution (`resolve_type`/`function_signature`), the
/// same role it already played for a class with no superclass.
fn build_flattened_class_info(
  name: &str,
  class_defs: &HashMap<String, &ClassDef>,
  classes: &HashMap<String, ClassInfo>,
  interfaces: &HashMap<String, InterfaceInfo>,
) -> Result<ClassInfo, Diagnostic> {
  let chain = resolve_chain(name, classes)?;
  let mut fields: HashMap<String, Type> = HashMap::new();
  let mut field_owner: HashMap<String, String> = HashMap::new();
  let mut methods: HashMap<String, FunctionSig> = HashMap::new();
  let mut generic_methods: HashMap<String, GenericMethodSig> = HashMap::new();
  // Plan 88's Decision log: an ordinary (non-generic) class has no type
  // parameter of its own to substitute — an empty `subst` map makes
  // `generic_method_signature` a no-op class-level substitution,
  // leaving only the method's own free type parameter symbolic.
  let no_class_subst: HashMap<&str, &TypeExpr> = HashMap::new();
  // Plan 89's Decision log: `name`'s own `implements Iterable[Int64]`
  // clause (if any) binds the interface's own type parameter to a
  // concrete type for `name`'s OWN declared methods only — never
  // applied to an ancestor's methods walked further down in this same
  // loop (`class_name == name` gates it below), since an ancestor's
  // declaration has no relationship to `name`'s own `implements` at all.
  let iface_subst = interface_type_param_subst(class_defs[name], interfaces);
  for class_name in &chain {
    let c = class_defs
      .get(class_name.as_str())
      .expect("every name in a resolved chain came from a registered ClassDef");
    let subst = if class_name == name {
      &iface_subst
    } else {
      &no_class_subst
    };
    for f in &c.fields {
      if let Some(owner) = field_owner.get(&f.name) {
        return Err(Diagnostic::new(
          format!(
            "field `{}` already declared in superclass `{owner}`",
            f.name
          ),
          (0, 0),
        ));
      }
      fields.insert(f.name.clone(), resolve_type(&f.ty, classes)?);
      field_owner.insert(f.name.clone(), class_name.clone());
    }
    for m in &c.methods {
      // Plan 61's Decision log: `comptime` is legal only on a top-level
      // function — mirrors `type_params`'s own top-level-only precedent
      // (`function_signature`'s own "generic methods are not supported"
      // check immediately below), but checked here directly rather than
      // inside `function_signature` itself, since (unlike `type_params`)
      // a legitimate top-level `comptime` function DOES reach
      // `function_signature` (only a *generic* top-level function is
      // filtered out before the call) — an unconditional check inside
      // `function_signature` would incorrectly reject that legitimate
      // top-level case too.
      if m.is_comptime {
        return Err(Diagnostic::new(
          format!(
            "`comptime` functions must be top-level — found on class method `{class_name}#{}`",
            m.name
          ),
          (0, 0),
        ));
      }
      // Plan 88's Decision log: a method's own non-empty `type_params`
      // routes to `generic_methods` instead of ever reaching
      // `function_signature` (whose own "generic methods are not
      // supported yet" rejection still stands for MODULE methods and
      // top-level functions — only the class-method path routes around
      // it here). No override-signature-match check applies to a
      // generic method — an override with a genuinely different
      // signature is a real, disclosed gap, matching this same
      // function's existing lack of ANY generic-aware override check.
      if !m.type_params.is_empty() {
        let sig = generic_method_signature(class_name, m, subst, classes)?;
        generic_methods.insert(m.name.clone(), sig);
        continue;
      }
      let sig = function_signature_with_subst(m, subst, classes)?;
      // `initialize` is exempt from the invariant-signature override
      // check: every class's constructor is inherently class-specific
      // (this plan's own worked example has `Dog::initialize` take an
      // extra `breed_code` param `Animal::initialize` doesn't) — it's
      // not really "overriding" a shared method the way `describe` is,
      // it's each class's own independent construction signature.
      if m.name != "initialize" {
        if let Some(existing) = methods.get(&m.name) {
          if existing.params != sig.params || existing.return_type != sig.return_type {
            return Err(Diagnostic::new(
              format!(
                "method `{}` override in `{class_name}` has a different signature than the method it overrides: expected {:?} -> {:?}, found {:?} -> {:?}",
                m.name, existing.params, existing.return_type, sig.params, sig.return_type
              ),
              (0, 0),
            ));
          }
        }
      }
      methods.insert(m.name.clone(), sig);
    }
  }
  Ok(ClassInfo {
    fields,
    methods,
    is_module: false,
    superclass: class_defs[name].superclass.clone(),
    implements: class_defs[name].implements.clone(),
    enum_variants: None,
    is_actor: false,
    generic_methods,
  })
}

/// A module's method table — built via the exact same `function_signature`
/// every free function's signature already goes through (plan 12's
/// Decision log: a module method type-checks like a free function,
/// because it is one, just namespaced). Always empty `fields`, never a
/// `superclass` (modules don't inherit).
fn module_info(
  m: &ModuleDef,
  classes: &HashMap<String, ClassInfo>,
) -> Result<ClassInfo, Diagnostic> {
  let mut methods = HashMap::new();
  for f in &m.methods {
    // Plan 61's Decision log: see `build_flattened_class_info`'s own
    // identical check for the full rationale.
    if f.is_comptime {
      return Err(Diagnostic::new(
        format!(
          "`comptime` functions must be top-level — found on module method `{}#{}`",
          m.name, f.name
        ),
        (0, 0),
      ));
    }
    methods.insert(f.name.clone(), function_signature(f, classes)?);
  }
  Ok(ClassInfo {
    fields: HashMap::new(),
    methods,
    is_module: true,
    superclass: None,
    implements: None,
    enum_variants: None,
    is_actor: false,
    generic_methods: HashMap::new(),
  })
}

/// An actor's field/method table (plan 54's Decision log: mirrors
/// `module_info`'s own directness — a single-pass, non-chain-walking
/// function, since `ActorDef` has no superclass to flatten against, and
/// its grammar production already forbids one structurally). Unlike a
/// module, an actor DOES have real fields, populated exactly like a
/// class's own (`resolve_type` on each declared field type, no
/// inheritance merge needed since there is only ever one "ancestor":
/// the actor itself).
fn actor_info(a: &ActorDef, classes: &HashMap<String, ClassInfo>) -> Result<ClassInfo, Diagnostic> {
  let mut fields = HashMap::new();
  for f in &a.fields {
    fields.insert(f.name.clone(), resolve_type(&f.ty, classes)?);
  }
  let mut methods = HashMap::new();
  for m in &a.methods {
    // Plan 55's Decision log: every actor method except `initialize`
    // must declare no return type (defaults to `"Void"` when omitted —
    // `crates/emerald-parser/src/grammar.lalrpop`'s own `FuncDef`
    // production) — a cross-actor call is genuinely asynchronous now,
    // so a real return value can never come back from one synchronously
    // (mirrors Pony's "behaviours always return `None`" rule).
    // `initialize` is exempt: `.spawn` still calls it directly and
    // synchronously, before any mailbox exists to send anything to.
    if m.name != "initialize" && m.return_type != "Void" {
      return Err(Diagnostic::new(
        format!(
          "actor method `{}` must not declare a return type — cross-actor calls are asynchronous \
           and cannot return a value synchronously (only `initialize` is exempt)",
          m.name
        ),
        (0, 0),
      ));
    }
    // Plan 61's Decision log: `MethodDef*` (the same grammar production
    // `ClassDef` uses) is reachable here too, so a `comptime`-marked
    // actor method parses — rejected the same way class/module methods
    // are, a real extension beyond the task brief's own literal "class
    // method or module method" wording, since actor methods are equally
    // nonsensical `comptime` targets (asynchronous behaviors, not
    // top-level pure functions).
    if m.is_comptime {
      return Err(Diagnostic::new(
        format!(
          "`comptime` functions must be top-level — found on actor method `{}#{}`",
          a.name, m.name
        ),
        (0, 0),
      ));
    }
    methods.insert(m.name.clone(), function_signature(m, classes)?);
  }
  Ok(ClassInfo {
    fields,
    methods,
    is_module: false,
    superclass: None,
    implements: None,
    enum_variants: None,
    is_actor: true,
    generic_methods: HashMap::new(),
  })
}

/// `Sub`/`Mul`/`Div`/`Rem` each apply the exact rule `Add` already
/// enforces (plan 18's Decision log): both operands must resolve to the
/// same numeric type, no implicit conversion.
#[allow(clippy::too_many_arguments)]
fn check_numeric_binop(
  op: &str,
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  // Plan 40's Decision log: `Sub`/`Mul`/`Div`/`Rem` all share this
  // function, so this one branch covers `-`/`*`/`/` (three of this
  // plan's eight scoped tokens) plus `%` (outside the plan's literal
  // list, but the plan's own Target state describes routing through
  // this shared function as-is — a class simply won't have a `%`
  // method, giving the same "no operator method" diagnostic as any
  // other undeclared one).
  if let Type::Class(class_name) = &lt {
    return resolve_class_operator(op, class_name, rhs, env, sigs, classes, self_fields, gctx);
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
  if lt != rt {
    // Plan 22's own concrete-proof diagnostic (Decision log): points at
    // `rhs`'s own span — the operand that disagrees with `lhs`'s
    // already-established type — not the whole binary expression.
    return Err(Diagnostic::new(
      format!(
        "type mismatch: `{op}` requires both operands to have the same type, found {lt:?} and {rt:?}"
      ),
      rhs.span,
    ));
  }
  if lt != Type::Int64 && lt != Type::Float64 {
    return Err(Diagnostic::new(
      format!("type `{lt:?}` does not support `{op}`"),
      lhs.span,
    ));
  }
  Ok(lt)
}

/// Plan 45's curated `String` intrinsic surface — a fixed name to
/// `(expected argument types, return type)` table, checked via the
/// existing generic `check_args` the same way an ordinary method call
/// already is. `None` for any name outside this fixed set (never a
/// general "look up a method on String" mechanism — see the plan's own
/// Decision log).
fn string_intrinsic_signature(method: &str) -> Option<(Vec<Type>, Type)> {
  match method {
    "length" => Some((vec![], Type::Int64)),
    "upcase" => Some((vec![], Type::String)),
    "downcase" => Some((vec![], Type::String)),
    "strip" => Some((vec![], Type::String)),
    "to_i" => Some((vec![], Type::Int64)),
    "to_f" => Some((vec![], Type::Float64)),
    // Ruby's own less-common `.slice(start, len)` method form —
    // `Expr::Index` is single-argument only, so a two-argument bracket
    // slice has no grammar shape to land in without a broader new
    // production (plan 45's Decision log).
    "slice" => Some((vec![Type::Int64, Type::Int64], Type::String)),
    // `Array[T]` carries no runtime length metadata — `.split_count`
    // is the companion scalar this representation limit forces (plan
    // 45's Decision log); neither is useful without the other.
    "split" => Some((vec![Type::String], Type::Array(Box::new(Type::String)))),
    "split_count" => Some((vec![Type::String], Type::Int64)),
    // Plan 59's Decision log: the cheaper, total (never-nil) direction
    // — an Emerald `String` is already guaranteed non-null, NUL-
    // terminated, real UTF-8, so nothing needs checking going out.
    "to_cstring" => Some((vec![], Type::CString)),
    _ => None,
  }
}

/// Plan 40's Decision log: routes an operator token (`"+"`, `"-"`,
/// `"*"`, `"/"`, `"=="`, `"[]"`, `"[]="`, ...) on a class-typed operand
/// to that class's own declared operator method — mirrors `Expr::
/// MethodCall`'s own existing arm exactly (a plain `HashMap` lookup by
/// method name, then the existing `check_args` for arity/type
/// checking against the method's own declared signature), not a new
/// dispatch mechanism.
#[allow(clippy::too_many_arguments)]
fn resolve_class_operator(
  op: &str,
  class_name: &str,
  rhs: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let info = classes
    .get(class_name)
    .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`"), rhs.span))?;
  let sig = info.methods.get(op).ok_or_else(|| {
    Diagnostic::new(
      format!("class `{class_name}` has no operator method `{op}`"),
      rhs.span,
    )
  })?;
  check_args(
    op,
    std::slice::from_ref(rhs),
    &sig.params,
    env,
    sigs,
    classes,
    self_fields,
    gctx,
  )?;
  Ok(sig.return_type.clone())
}

/// `&&`/`||` require both operands `Boolean`, return `Boolean`.
#[allow(clippy::too_many_arguments)]
fn check_boolean_binop(
  op: &str,
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  if lt != Type::Boolean {
    return Err(Diagnostic::new(
      format!("`{op}` requires a Boolean left operand, found {lt:?}"),
      lhs.span,
    ));
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
  if rt != Type::Boolean {
    return Err(Diagnostic::new(
      format!("`{op}` requires a Boolean right operand, found {rt:?}"),
      rhs.span,
    ));
  }
  Ok(Type::Boolean)
}

/// `&`/`|`/`^`/`<<`/`>>` (plan 28's Decision log): `Int64`-only, unlike
/// `check_numeric_binop`'s `Int64`-or-`Float64` — bitwise operators have
/// no `Float64` semantics in this language.
#[allow(clippy::too_many_arguments)]
fn check_bitwise_binop(
  op: &str,
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  if lt != Type::Int64 {
    return Err(Diagnostic::new(
      format!("`{op}` requires an Int64 left operand, found {lt:?}"),
      lhs.span,
    ));
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
  if rt != Type::Int64 {
    return Err(Diagnostic::new(
      format!("`{op}` requires an Int64 right operand, found {rt:?}"),
      rhs.span,
    ));
  }
  Ok(Type::Int64)
}

/// Plan 42 (enumerable stdlib) — real, disclosed simplification from
/// this plan's own literal design: `Array[T]`/`Hash[K,V]` get eight
/// intrinsic methods (`each`, `map`, `select`/`filter`, `reduce`/
/// `inject`, `each_with_index`, `count`, `sum`, `sort`) implemented
/// directly against these two built-in types, rather than a genuinely
/// generic `Iterable[T]` interface layered onto plan 41's interface/
/// monomorphization mechanism. Verified this session: plan 41's real,
/// shipped `resolve_type` has no bracketed `Proc[...]` annotation
/// parsing at all (only the bare `"Proc"` keyword) — the plan's own
/// assumed prerequisite for `Iterable[T]`'s own `each(block: Proc[T,
/// Void])` signature was never actually built. Reproducing that
/// machinery faithfully would mean adding a second, parallel generics-
/// annotation-parsing feature to plan 41's own contract on this plan's
/// behalf — real, substantial, cross-cutting work this plan does not
/// take on unannounced. This simplification still satisfies every
/// acceptance criterion this plan's own leaves state (none of them
/// actually test a *user-defined* class implementing `Iterable[T]` —
/// every AC is `Array[Int64]`/`Hash[Int64,Int64]` concrete behavior),
/// at the real, disclosed cost the call site's own comment names: a
/// user class that declares its own method named one of these ten is
/// shadowed. `sort`'s own real, disclosed narrowing from the plan's
/// stated `Comparable`-bounded design: only `Int64`/`Float64`/`String`
/// element types are supported (natively ordered via `Expr::Compare`)
/// — a real `<=>`-dispatching fork for a user `Comparable`-implementing
/// class needs the same missing generics-annotation machinery
/// `Iterable[T]` does, deferred for the identical reason.
#[allow(clippy::too_many_arguments)]
fn check_enumerable_call(
  recv_ty: &Type,
  method: &str,
  args: &[Spanned<Expr>],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
  span: (usize, usize),
) -> Result<Type, Diagnostic> {
  // `count` is the only method both Array and Hash support.
  //
  // Plan 70 (enumerable stdlib completion): `.count do |x| ... end` — an
  // optional predicate `Proc` (Boolean-returning), counting only the
  // elements/pairs it accepts, real new scope beyond plan 42's original
  // arity-0-only `.count` (a plain O(1) header read, unchanged and
  // still the fast path taken when no predicate is passed).
  if method == "count" {
    if args.is_empty() {
      return Ok(Type::Int64);
    }
    let [proc_arg] = args else {
      return Err(Diagnostic::new(
        format!(
          "`.count` takes 0 or 1 arguments (an optional predicate Proc), found {}",
          args.len()
        ),
        span,
      ));
    };
    let elem_ty = match recv_ty {
      Type::Array(elem) => (**elem).clone(),
      Type::Hash(k, v) => Type::Pair(k.clone(), v.clone()),
      _ => unreachable!("caller already checked recv_ty is Array or Hash"),
    };
    let result_ty = check_enumerable_proc_arg(
      proc_arg,
      std::slice::from_ref(&elem_ty),
      env,
      sigs,
      classes,
      self_fields,
      gctx,
    )?;
    if result_ty != Type::Boolean {
      return Err(Diagnostic::new(
        format!("`.count`'s predicate Proc must return Boolean, found {result_ty:?}"),
        proc_arg.span,
      ));
    }
    return Ok(Type::Int64);
  }

  if method == "each" {
    let [proc_arg] = args else {
      return Err(Diagnostic::new(
        format!(
          "`.each` expects exactly 1 argument (a Proc), found {}",
          args.len()
        ),
        span,
      ));
    };
    let elem_ty = match recv_ty {
      Type::Array(elem) => (**elem).clone(),
      Type::Hash(k, v) => Type::Pair(k.clone(), v.clone()),
      _ => unreachable!("caller already checked recv_ty is Array or Hash"),
    };
    check_enumerable_proc_arg(proc_arg, &[elem_ty], env, sigs, classes, self_fields, gctx)?;
    return Ok(Type::Void);
  }

  // Plan 70 (enumerable stdlib completion): `map`/`reduce`/`inject`/
  // `each_with_index` on `Hash[K,V]` — the "K,V-appropriate subset"
  // this plan's own leaf describes, iterating `Pair[K,V]` elements
  // exactly the way `.each`/`.count` above already do. `sum`/`sort`
  // stay Array-only, deliberately not extended here: a `Pair` has no
  // natural sum (nothing to add two pairs together into) or total
  // order (`sort`'s own Decision log already narrows ordering to
  // `Int64`/`Float64` for the SAME reason a real `Pair` ordering would
  // need — a `Comparable`-dispatching fork this plan's Decision log
  // declines to add). `select`/`filter` are ALSO deliberately left
  // Array-only, for a real, different, and more fundamental reason
  // found implementing this plan: their only sensible result type is
  // `Array[Pair[K,V]]`, but `grammar.lalrpop`'s `TypeName` rule can
  // only parse `Array[<a bare Ident>]` — a nested compound element
  // type like `Pair[K,V]` inside `Array[...]` cannot be written in
  // this language's concrete syntax at all (confirmed against the real
  // parser: `x: Array[Pair[Int64, Int64]] = ...` is a real parse
  // error, not a hypothetical one), so a `Let` could never even name
  // the result — and no chaining is allowed either (this plan's own
  // Decision log), so there is no other way to consume it. Every
  // method kept in this list produces a directly nameable type
  // (`Array[Elem]` for `map`, the accumulator's own type for
  // `reduce`/`inject`, `Void` for `each_with_index`). Falls through to
  // the existing Array-only rejection below for `select`/`filter`/
  // `sum`/`sort` (and any future method), which already produces an
  // accurate "is only supported on Array[T], found Hash(...)"
  // diagnostic without needing a second copy of that message here.
  if let Type::Hash(k_ty, v_ty) = recv_ty {
    if matches!(method, "map" | "reduce" | "inject" | "each_with_index") {
      let elem_ty = Type::Pair(k_ty.clone(), v_ty.clone());
      return match method {
        "map" => {
          let [proc_arg] = args else {
            return Err(Diagnostic::new(
              format!(
                "`.map` expects exactly 1 argument (a Proc), found {}",
                args.len()
              ),
              span,
            ));
          };
          let result_ty =
            check_enumerable_proc_arg(proc_arg, &[elem_ty], env, sigs, classes, self_fields, gctx)?;
          Ok(Type::Array(Box::new(result_ty)))
        }
        "reduce" | "inject" => {
          let [initial, proc_arg] = args else {
            return Err(Diagnostic::new(
              format!(
                "`.{method}` expects exactly 2 arguments (an initial value and a Proc), found {}",
                args.len()
              ),
              span,
            ));
          };
          let acc_ty = infer_expr_type(initial, env, sigs, classes, self_fields, gctx)?;
          let result_ty = check_enumerable_proc_arg(
            proc_arg,
            &[acc_ty.clone(), elem_ty],
            env,
            sigs,
            classes,
            self_fields,
            gctx,
          )?;
          if result_ty != acc_ty {
            return Err(Diagnostic::new(
              format!(
                "`.{method}`'s Proc must return the same type as the initial value ({acc_ty:?}), found {result_ty:?}"
              ),
              proc_arg.span,
            ));
          }
          Ok(acc_ty)
        }
        "each_with_index" => {
          let [proc_arg] = args else {
            return Err(Diagnostic::new(
              format!(
                "`.each_with_index` expects exactly 1 argument (a Proc), found {}",
                args.len()
              ),
              span,
            ));
          };
          check_enumerable_proc_arg(
            proc_arg,
            &[elem_ty, Type::Int64],
            env,
            sigs,
            classes,
            self_fields,
            gctx,
          )?;
          Ok(Type::Void)
        }
        other => unreachable!("just matched method against a fixed set, found `{other}`"),
      };
    }
  }

  // Every remaining method (`map`/`select`/`filter`/`reduce`/`inject`/
  // `each_with_index`/`sum`/`sort`) is Array-only.
  let Type::Array(elem_ty) = recv_ty else {
    return Err(Diagnostic::new(
      format!("`.{method}` is only supported on Array[T], found {recv_ty:?}"),
      span,
    ));
  };
  let elem_ty = (**elem_ty).clone();

  match method {
    "map" => {
      let [proc_arg] = args else {
        return Err(Diagnostic::new(
          format!(
            "`.map` expects exactly 1 argument (a Proc), found {}",
            args.len()
          ),
          span,
        ));
      };
      let result_ty =
        check_enumerable_proc_arg(proc_arg, &[elem_ty], env, sigs, classes, self_fields, gctx)?;
      Ok(Type::Array(Box::new(result_ty)))
    }
    "select" | "filter" => {
      let [proc_arg] = args else {
        return Err(Diagnostic::new(
          format!(
            "`.{method}` expects exactly 1 argument (a Proc), found {}",
            args.len()
          ),
          span,
        ));
      };
      let result_ty = check_enumerable_proc_arg(
        proc_arg,
        std::slice::from_ref(&elem_ty),
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if result_ty != Type::Boolean {
        return Err(Diagnostic::new(
          format!("`.{method}`'s Proc must return Boolean, found {result_ty:?}"),
          proc_arg.span,
        ));
      }
      Ok(Type::Array(Box::new(elem_ty)))
    }
    "reduce" | "inject" => {
      let [initial, proc_arg] = args else {
        return Err(Diagnostic::new(
          format!(
            "`.{method}` expects exactly 2 arguments (an initial value and a Proc), found {}",
            args.len()
          ),
          span,
        ));
      };
      let acc_ty = infer_expr_type(initial, env, sigs, classes, self_fields, gctx)?;
      let result_ty = check_enumerable_proc_arg(
        proc_arg,
        &[acc_ty.clone(), elem_ty],
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if result_ty != acc_ty {
        return Err(Diagnostic::new(
          format!(
            "`.{method}`'s Proc must return the same type as the initial value ({acc_ty:?}), found {result_ty:?}"
          ),
          proc_arg.span,
        ));
      }
      Ok(acc_ty)
    }
    "each_with_index" => {
      let [proc_arg] = args else {
        return Err(Diagnostic::new(
          format!(
            "`.each_with_index` expects exactly 1 argument (a Proc), found {}",
            args.len()
          ),
          span,
        ));
      };
      check_enumerable_proc_arg(
        proc_arg,
        &[elem_ty, Type::Int64],
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(Type::Void)
    }
    "sum" => {
      if !args.is_empty() {
        return Err(Diagnostic::new(
          format!("`.sum` takes no arguments, found {}", args.len()),
          span,
        ));
      }
      if elem_ty != Type::Int64 && elem_ty != Type::Float64 {
        return Err(Diagnostic::new(
          format!("`.sum` requires an Int64 or Float64 element type, found Array[{elem_ty:?}]"),
          span,
        ));
      }
      Ok(elem_ty)
    }
    "sort" => {
      if !args.is_empty() {
        return Err(Diagnostic::new(
          format!("`.sort` takes no arguments, found {}", args.len()),
          span,
        ));
      }
      // Plan 42's own real, disclosed narrowing from its stated
      // `Comparable`-bounded design: `String` is dropped from the
      // allowed set (unlike `Int64`/`Float64`, this backend has no
      // ordering-comparison runtime helper for `String` at all, and
      // every one of this plan's own worked/tested `sort` examples
      // only ever uses `Array[Int64]`).
      if elem_ty != Type::Int64 && elem_ty != Type::Float64 {
        return Err(Diagnostic::new(
          format!("`.sort` requires an Int64 or Float64 element type, found Array[{elem_ty:?}]"),
          span,
        ));
      }
      Ok(Type::Array(Box::new(elem_ty)))
    }
    other => {
      unreachable!("caller already filtered to the known enumerable method set, found `{other}`")
    }
  }
}

/// Type-checks one `Proc`-typed argument passed to an intrinsic Array/
/// Hash method (`check_enumerable_call`'s own doc comment has the full
/// "why a hard-coded arm" rationale) — `expected_param_types` is each
/// positional parameter's REQUIRED type, in order (arity AND each type
/// checked against it); returns the Proc's own declared return type.
///
/// Real, disclosed design correction, found and fixed this session
/// (superseding this function's own first-drafted, now-removed
/// `check_enumerable_block`, which type-checked an INLINE block
/// literal's body directly): plan 34's own trailing-`{ |params| ... }`
/// block-literal syntax (since plan 87, `do |params| ... end`) attaches
/// ONLY to a bare, statement-initial call (`grammar.lalrpop`'s own
/// `StmtPrimaryExpr` — verified this session; `PrimaryExpr`, the
/// nonterminal actually reachable from a `Let`'s RHS or a nested call
/// argument, deliberately did NOT gain a trailing block at the time this
/// comment was first written, a real, pre-existing LALR(1) conflict
/// with `HashLit` the grammar's own comment documented — plan 70 later
/// lifted that restriction, and plan 87 changed the delimiter again) —
/// so `evens: Array[Int64] = arr.select() { |x: Int64| ... }` **did not
/// parse at all** in this compiler's grammar at the time. The only way
/// to pass "a function value" to
/// an ordinary call argument position here is plan 10's pre-existing
/// mechanism: bind a lambda to a top-level `Proc`-typed `Let` first
/// (`is_even: Proc = ->(x: Int64) -> Boolean { x % 2 == 0 }`), then
/// pass that name as a plain `Expr::Ident` argument — which is exactly
/// what `arr.select(is_even)` already is, with zero new grammar. This
/// function accepts anything typed `Type::Proc(...)`, matching that
/// existing mechanism precisely, and simply reuses its own already-
/// inferred signature — no separate block-body walk needed at all.
fn check_enumerable_proc_arg(
  arg: &Spanned<Expr>,
  expected_param_types: &[Type],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let arg_ty = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
  let Type::Proc(param_types, return_type) = &arg_ty else {
    return Err(Diagnostic::new(
      format!(
        "expected a Proc (a name bound via `name: Proc = ->(...) -> R {{ ... }}`), found {arg_ty:?}"
      ),
      arg.span,
    ));
  };
  if param_types.as_slice() != expected_param_types {
    return Err(Diagnostic::new(
      format!(
        "Proc argument has parameter types {param_types:?}, expected {expected_param_types:?}"
      ),
      arg.span,
    ));
  }
  Ok((**return_type).clone())
}

/// `self_fields` is `Some(&class.fields)` while checking a method body,
/// `None` everywhere else — gates `@field` legality (plan 08 AC4).
#[allow(clippy::too_many_arguments)]
fn infer_expr_type(
  expr: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  match &expr.node {
    // Plan 73's Decision log: a bare, zero-arg variant reference —
    // `Option[T]`'s own `None` — parses as an ordinary `Expr::Ident` (no
    // grammar changes at all: `None` is not a reserved keyword, just an
    // identifier that happens to name a nullary variant), checked here
    // only once `env` itself has no binding for `name` (an ordinary
    // local always wins — the same "no reserved word" precedent this
    // fallback relies on). Real ambiguity between multiple nullary
    // candidates (two different `Option[T]` instantiations both
    // declaring `None`) can't be resolved from arguments (there are
    // none) — reached only when `check_enum_variant_construction`'s own
    // expected-type-providing positions didn't already resolve it, so
    // this is a genuine, disclosed diagnostic, not a silent guess (the
    // same limitation Rust's own `None` inference has outside an
    // expected-type position).
    Expr::Ident(name) if !env.contains_key(name) && !find_all_variants(name, classes).is_empty() => {
      let candidates = find_all_variants(name, classes);
      let nullary: Vec<&(String, Vec<Type>)> =
        candidates.iter().filter(|(_, ft)| ft.is_empty()).collect();
      match nullary.len() {
        1 => Ok(Type::Enum(nullary[0].0.clone())),
        0 => Err(Diagnostic::new(
          format!("`{name}` is not a nullary variant — it takes arguments (write `{name}(...)`)"),
          expr.span,
        )),
        _ => {
          let names: Vec<&str> = nullary.iter().map(|(n, _)| n.as_str()).collect();
          Err(Diagnostic::new(
            format!(
              "ambiguous construction `{name}` — matches more than one instantiation: `{}` — add a declared type here (a `let`, `return`, or function argument with an explicit `Option[T]` type) to disambiguate",
              names.join("`, `")
            ),
            expr.span,
          ))
        }
      }
    }
    Expr::Ident(name) => env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), expr.span)),
    Expr::Int(_) => Ok(Type::Int64),
    Expr::Float(_) => Ok(Type::Float64),
    // Plan 19: a real `Type::String` value at last (the annotation
    // already resolved; nothing could ever produce one before this).
    Expr::StringLit(_) => Ok(Type::String),
    // Plan 44: `Expr::Compare`'s existing generic `lt != rt` rule
    // already covers `Symbol == Symbol`/`!=` for free the moment this
    // produces a real `Type::Symbol` — no `Compare` arm change needed,
    // the identical "for free" shape plan 19 already established for
    // `String`.
    Expr::SymbolLit(_) => Ok(Type::Symbol),
    // Plan 36: a compiler-known stringification set only — `Int64`,
    // `Float64`, `String`, `Boolean` — not a generic, user-extensible
    // `to_s`/`Display` protocol (Decision log: no interface/protocol
    // mechanism exists yet to type-check "the receiver's declared type
    // has a method named `to_s`" against arbitrary future classes).
    // Always produces `Type::String` overall.
    Expr::Interpolate(parts) => {
      for part in parts {
        if let StringPart::Expr(e) = part {
          let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
          if !matches!(
            t,
            Type::Int64 | Type::Float64 | Type::String | Type::Boolean
          ) {
            return Err(Diagnostic::new(
              format!(
                "type `{t:?}` cannot be interpolated into a string — only Int64, Float64, String, and Boolean are supported"
              ),
              e.span,
            ));
          }
        }
      }
      Ok(Type::String)
    }
    // Plan 40's Decision log: checked *before* the existing `lt != rt`/
    // `lt ∉ {Int64, Float64, String}` rule below, which is otherwise
    // completely unchanged — `1 + 2`/`1.0 + 2.0`/`"a" + "b"` compile to
    // the identical instructions as before this plan.
    Expr::Add(lhs, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
      if let Type::Class(class_name) = &lt {
        return resolve_class_operator("+", class_name, rhs, env, sigs, classes, self_fields, gctx);
      }
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
      if lt != rt {
        // Plan 22's own concrete-proof diagnostic (Decision log /
        // AC1): `rhs`'s own span, not `lhs`'s and not the whole `Add`
        // expression — the operand that disagrees with `lhs`'s
        // already-established type.
        return Err(Diagnostic::new(
          format!(
            "type mismatch: `+` requires both operands to have the same type, found {lt:?} and {rt:?}"
          ),
          rhs.span,
        ));
      }
      // Plan 19: `+` on two `String`s concatenates — this plan owns all
      // of `Add`'s `Type::String` case (the separate operators plan is
      // numeric/boolean-only and never touches `Add`/`String`).
      if lt != Type::Int64 && lt != Type::Float64 && lt != Type::String {
        return Err(Diagnostic::new(
          format!("type `{lt:?}` does not support `+`"),
          lhs.span,
        ));
      }
      Ok(lt)
    }
    Expr::Sub(lhs, rhs) => {
      check_numeric_binop("-", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Mul(lhs, rhs) => {
      check_numeric_binop("*", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Div(lhs, rhs) => {
      check_numeric_binop("/", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Rem(lhs, rhs) => {
      check_numeric_binop("%", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Neg(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Int64 && t != Type::Float64 {
        return Err(Diagnostic::new(
          format!("type `{t:?}` does not support unary `-`"),
          e.span,
        ));
      }
      Ok(t)
    }
    Expr::Not(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Boolean {
        return Err(Diagnostic::new(
          format!("`!` requires a Boolean operand, found {t:?}"),
          e.span,
        ));
      }
      Ok(Type::Boolean)
    }
    Expr::And(lhs, rhs) => {
      check_boolean_binop("&&", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Or(lhs, rhs) => {
      check_boolean_binop("||", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    // Plan 28: Int64-only bitwise operators.
    Expr::BitAnd(lhs, rhs) => {
      check_bitwise_binop("&", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitOr(lhs, rhs) => {
      check_bitwise_binop("|", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitXor(lhs, rhs) => {
      check_bitwise_binop("^", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Shl(lhs, rhs) => {
      check_bitwise_binop("<<", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Shr(lhs, rhs) => {
      check_bitwise_binop(">>", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitNot(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Int64 {
        return Err(Diagnostic::new(
          format!("`~` requires an Int64 operand, found {t:?}"),
          e.span,
        ));
      }
      Ok(Type::Int64)
    }
    // Plan 40's Decision log: a real, verified gap this plan closes —
    // two same-class instances used to "type-check" here with no
    // operator involved at all (codegen then failed with a generic
    // error), since this arm previously had no restriction beyond
    // `lt == rt`. `==`/`!=` now require the class to declare `==`
    // (`!=` reuses the identical lookup and negates the result — Ruby's
    // own default); any other `CompareOp` on a class operand is an
    // explicit diagnostic rather than a confusing late codegen failure
    // (Ruby derives `<`/`>`/`<=`/`>=` from `<=>` via `Comparable`,
    // which this compiler has no mixin mechanism to reproduce — that
    // derivation is plan 41's job, once `<=>` is callable).
    Expr::Compare(lhs, op, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
      if let Type::Class(class_name) = &lt {
        return match op {
          CompareOp::Eq | CompareOp::Ne => {
            let ret =
              resolve_class_operator("==", class_name, rhs, env, sigs, classes, self_fields, gctx)?;
            if ret != Type::Boolean {
              return Err(Diagnostic::new(
                format!("class `{class_name}`'s `==` method must return Boolean, found {ret:?}"),
                expr.span,
              ));
            }
            Ok(Type::Boolean)
          }
          _ => Err(Diagnostic::new(
            format!(
              "ordering comparison `{op:?}` is not supported on class `{class_name}` — define `<=>`, not a direct `{op:?}` overload"
            ),
            expr.span,
          )),
        };
      }
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
      if lt != rt {
        return Err(Diagnostic::new(
          format!(
            "type mismatch: `{op:?}` requires both operands to have the same type, found {lt:?} and {rt:?}"
          ),
          rhs.span,
        ));
      }
      Ok(Type::Boolean)
    }
    // `puts` is a compiler intrinsic, not an overloaded function (locks
    // spec/SEMANTICS.md §3's "no overloading in v1") — it accepts exactly
    // one Int64 or Float64 argument, checked here directly rather than via
    // a `FunctionSig` in `sigs` (see plan 08's Decision log).
    Expr::Call(name, args) if name == "puts" => {
      if args.len() != 1 {
        // Arity is a property of the whole call (Decision log), not
        // any one argument — the call's own span, not a per-arg one.
        return Err(Diagnostic::new(
          format!("`puts` expects 1 argument, found {}", args.len()),
          expr.span,
        ));
      }
      let arg_ty = infer_expr_type(&args[0], env, sigs, classes, self_fields, gctx)?;
      if arg_ty != Type::Int64 && arg_ty != Type::Float64 && arg_ty != Type::String {
        return Err(Diagnostic::new(
          format!("`puts` does not support type {arg_ty:?}"),
          args[0].span,
        ));
      }
      Ok(Type::Void)
    }
    // Plan 45's Decision log: unlike `puts`, `gets` must be usable as
    // an expression (`line: String = gets()`) — checked the same way
    // `puts` is, directly here, not via a `FunctionSig` in `sigs`.
    Expr::Call(name, args) if name == "gets" => {
      if !args.is_empty() {
        return Err(Diagnostic::new(
          format!("`gets` expects 0 arguments, found {}", args.len()),
          expr.span,
        ));
      }
      Ok(Type::String)
    }
    // Plan 47's Decision log: `assert`/`assert_eq` are recognized by
    // literal call name, exactly the mechanism `puts`/`gets` already
    // use above — not new `Stmt` variants. The trailing argument is
    // always a real `Expr::StringLit("file:line")` by the time sema
    // ever sees it (`emerald_parser::parse_named`'s own post-parse
    // rewrite already replaced the grammar's raw-offset placeholder),
    // so it needs no special-casing here beyond the arity check.
    Expr::Call(name, args) if name == "assert" => {
      if args.len() != 2 {
        return Err(Diagnostic::new(
          format!(
            "`assert` expects 1 argument, found {}",
            args.len().saturating_sub(1)
          ),
          expr.span,
        ));
      }
      let cond_ty = infer_expr_type(&args[0], env, sigs, classes, self_fields, gctx)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(
          format!("`assert` expects a Boolean condition, found {cond_ty:?}"),
          args[0].span,
        ));
      }
      Ok(Type::Void)
    }
    // Not new sema logic — the existing `==`-comparison checker,
    // invoked on a synthetic `Expr::Compare(expected, Eq, actual)`
    // node, keeping only its `Ok(())`/`Err(Diagnostic)` (the
    // `Type::Boolean` it returns on success is discarded). This is
    // also `assert_eq`'s real scope: exactly the operand types `==`
    // already supports, not classes/arrays/hashes.
    Expr::Call(name, args) if name == "assert_eq" => {
      if args.len() != 3 {
        return Err(Diagnostic::new(
          format!(
            "`assert_eq` expects 2 arguments, found {}",
            args.len().saturating_sub(1)
          ),
          expr.span,
        ));
      }
      let compare = Spanned::synthetic(Expr::Compare(
        Box::new(args[0].clone()),
        CompareOp::Eq,
        Box::new(args[1].clone()),
      ));
      infer_expr_type(&compare, env, sigs, classes, self_fields, gctx)?;
      Ok(Type::Void)
    }
    // Plan 41's Decision log: call-site checking is a separate, later
    // pass from body-checking, and only it ever touches a real concrete
    // type — checked before the ordinary `sigs.get(name)` fallback below
    // since a generic function's own name is never present in `sigs` at
    // all (the registration pass skips it).
    Expr::Call(name, args) if gctx.generic_sigs.contains_key(name) => {
      let g = &gctx.generic_sigs[name];
      let arg_types = args
        .iter()
        .map(|a| infer_expr_type(a, env, sigs, classes, self_fields, gctx))
        .collect::<Result<Vec<_>, _>>()?;
      let mut concrete: Option<Type> = None;
      for (i, (_, raw)) in g.params_raw.iter().enumerate() {
        if raw.as_named() != Some(g.type_param.as_str()) {
          continue;
        }
        let Some(actual) = arg_types.get(i) else {
          return Err(Diagnostic::new(
            format!(
              "`{name}` expects {} argument(s), found {}",
              g.params_raw.len(),
              args.len()
            ),
            expr.span,
          ));
        };
        match &concrete {
          None => concrete = Some(actual.clone()),
          Some(c) if c != actual => {
            return Err(Diagnostic::new(
              format!(
                "type parameter `{}` resolved inconsistently in call to `{name}`: `{c:?}` at an earlier argument, `{actual:?}` at argument {}",
                g.type_param,
                i + 1
              ),
              args[i].span,
            ));
          }
          Some(_) => {}
        }
      }
      let concrete = concrete.ok_or_else(|| {
        Diagnostic::new(
          format!(
            "internal error: generic function `{name}` never uses its own type parameter `{}`",
            g.type_param
          ),
          expr.span,
        )
      })?;
      let bounds_display = g.bounds.join(" + ");
      let Type::Class(concrete_class) = &concrete else {
        return Err(Diagnostic::new(
          format!(
            "type parameter `{}` in call to `{name}` resolved to non-class type {concrete:?} — only a class implementing `{bounds_display}` is a legal generic argument",
            g.type_param
          ),
          expr.span,
        ));
      };
      let class_info = classes
        .get(concrete_class)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{concrete_class}`"), expr.span))?;
      // Plan 88's Decision log: `bounds` widened to a real conjunctive
      // set — EVERY bound must be satisfied, not just one. A class
      // still only ever declares ONE `implements` clause (unchanged,
      // out of this plan's scope), so a multi-bound requirement is
      // only ever satisfiable when that single bound set has exactly
      // one entry — a real, disclosed structural limit, not a bug: see
      // `check_generic_arg_bound`'s own identical per-bound check for
      // generic classes/enums.
      for bound in &g.bounds {
        if class_info.implements.as_ref().map(|(n, _)| n.as_str()) != Some(bound.as_str()) {
          return Err(Diagnostic::new(
            format!(
              "`{concrete_class}` does not implement `{bound}`, required by generic function `{name}`'s type parameter `{}`",
              g.type_param
            ),
            expr.span,
          ));
        }
      }
      let effective_params = g
        .params_raw
        .iter()
        .map(|(_, raw)| {
          if raw.as_named() == Some(g.type_param.as_str()) {
            Ok(concrete.clone())
          } else {
            resolve_type(raw, classes)
          }
        })
        .collect::<Result<Vec<_>, _>>()?;
      check_args(
        name,
        args,
        &effective_params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if g.return_type_raw.as_named() == Some(g.type_param.as_str()) {
        Ok(concrete)
      } else {
        resolve_type(&g.return_type_raw, classes)
      }
    }
    // Plan 53's Decision log: `is_valid_int`/`parse_digits` are
    // compiler-known intrinsic builtins, checked before the ordinary
    // function-signature lookup below the same way every other
    // hardcoded-name `Expr::Call` case in this file already is.
    Expr::Call(name, args) if name == "is_valid_int" || name == "parse_digits" => {
      if args.len() != 1 {
        return Err(Diagnostic::new(
          format!("`{name}` expects 1 argument, found {}", args.len()),
          expr.span,
        ));
      }
      let arg_ty = infer_expr_type(&args[0], env, sigs, classes, self_fields, gctx)?;
      if arg_ty != Type::String {
        return Err(Diagnostic::new(
          format!("`{name}` expects a String argument, found {arg_ty:?}"),
          args[0].span,
        ));
      }
      Ok(if name == "is_valid_int" {
        Type::Boolean
      } else {
        Type::Int64
      })
    }
    // Plan 55's Decision log: `current_thread_id()` is supplementary,
    // best-effort observability only (never consulted by any dispatch/
    // safety logic) — a compiler-known intrinsic builtin, the same
    // hardcoded-name convention `is_valid_int`/`parse_digits` above
    // already established.
    Expr::Call(name, args) if name == "current_thread_id" => {
      if !args.is_empty() {
        return Err(Diagnostic::new(
          format!(
            "`current_thread_id` expects 0 arguments, found {}",
            args.len()
          ),
          expr.span,
        ));
      }
      Ok(Type::Int64)
    }
    // Plan 52's Decision log: `Circle(2.0)` parses as an ordinary
    // `Expr::Call` (no new grammar production — the parser can't tell
    // "call a function" from "construct a variant" apart at all) — a
    // hit against the variant-owner lookup, checked before the
    // ordinary function-signature lookup below, resolves it as
    // construction instead. Registration-time collision checks in
    // `check_program` already guarantee a name is never both a
    // declared variant and a declared function.
    Expr::Call(name, args) if !find_all_variants(name, classes).is_empty() => {
      let mut candidates = find_all_variants(name, classes);
      // Plan 73's Decision log: an ordinary (non-generic) enum's variant
      // name is unique across the whole program (registration-time
      // collision check), so this is the overwhelmingly common case —
      // exactly one candidate, unchanged behavior from plan 52. A
      // generic enum instantiated more than once (`Option[Int64]` AND
      // `Option[String]` both used) genuinely has several candidates
      // sharing this variant name; a non-nullary variant (`Some(x)`)
      // disambiguates the same way an overloaded call would — by
      // checking which candidate's own field types the ARGUMENTS
      // actually match, entirely from context already at hand, no
      // expected-type threading needed. A nullary variant (`None`) has
      // no arguments to disambiguate from at all — this arm is only ever
      // reached for one when `check_enum_variant_construction` (an
      // expected-type-providing position) didn't already resolve it, so
      // real ambiguity here is a genuine diagnostic, not a silent guess.
      if candidates.len() > 1 {
        let arg_types: Vec<Type> = args
          .iter()
          .map(|a| infer_expr_type(a, env, sigs, classes, self_fields, gctx))
          .collect::<Result<Vec<_>, _>>()?;
        let matching: Vec<(String, Vec<Type>)> = candidates
          .iter()
          .filter(|(_, field_types)| {
            field_types.len() == arg_types.len()
              && field_types.iter().zip(&arg_types).all(|(f, a)| f == a)
          })
          .cloned()
          .collect();
        match matching.len() {
          1 => candidates = matching,
          0 => {
            let names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
            return Err(Diagnostic::new(
              format!(
                "`{name}({arg_types:?})` does not match any of `{}`'s variant `{name}` — candidates: `{}`",
                names.join("`, `"),
                names.join("`, `")
              ),
              expr.span,
            ));
          }
          _ => {
            let names: Vec<&str> = matching.iter().map(|(n, _)| n.as_str()).collect();
            return Err(Diagnostic::new(
              format!(
                "ambiguous construction `{name}(...)` — matches more than one instantiation: `{}` — add a declared type here (a `let`, `return`, or function argument with an explicit `Option[T]` type) to disambiguate",
                names.join("`, `")
              ),
              expr.span,
            ));
          }
        }
        let (enum_name, field_types) = candidates.into_iter().next().unwrap();
        check_args(
          name, args, &field_types, env, sigs, classes, self_fields, gctx,
        )?;
        return Ok(Type::Enum(enum_name));
      }
      let (enum_name, field_types) = candidates.into_iter().next().unwrap();
      check_args(
        name,
        args,
        &field_types,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(Type::Enum(enum_name))
    }
    Expr::Call(name, args) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`"), expr.span))?;
      // Plan 34: a trailing block literal desugars into an extra,
      // implicit `Expr::Lambda` argument at parse time (`grammar.
      // lalrpop`'s Decision log) — it's not one of `sig.params`'
      // ordinary positional arguments, so it's excluded here before the
      // ordinary arity/type check. Its own legality (present when
      // required, well-typed, `yield`-arity-compatible) is
      // `check_block_call_sites`' separate job, not this one's.
      let positional = if sig.block_param.is_some()
        && matches!(
          args.last(),
          Some(Spanned {
            node: Expr::Lambda { .. },
            ..
          })
        ) {
        &args[..args.len() - 1]
      } else {
        args.as_slice()
      };
      check_call_args(
        name,
        positional,
        sig,
        env,
        sigs,
        classes,
        self_fields,
        expr.span,
        gctx,
      )?;
      Ok(sig.return_type.clone())
    }
    // Plan 39's Decision log: resolved entirely at compile time by
    // name-to-position matching against `sig.param_names` — never a
    // runtime hash/dispatch. An unknown name, a duplicate name, or a
    // missing required (no-default) parameter is a real diagnostic
    // naming the offending keyword, not a panic.
    Expr::CallKw(name, kwargs) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`"), expr.span))?;
      let mut positional: Vec<Option<&Spanned<Expr>>> = vec![None; sig.param_names.len()];
      for (kw_name, kw_value) in kwargs {
        let Some(pos) = sig.param_names.iter().position(|p| p == kw_name) else {
          return Err(Diagnostic::new(
            format!("unrecognized keyword `{kw_name}` for `{name}`"),
            kw_value.span,
          ));
        };
        if positional[pos].is_some() {
          return Err(Diagnostic::new(
            format!("duplicate keyword `{kw_name}` in call to `{name}`"),
            kw_value.span,
          ));
        }
        positional[pos] = Some(kw_value);
      }
      for (i, slot) in positional.iter().enumerate() {
        if slot.is_none() && sig.defaults[i].is_none() {
          return Err(Diagnostic::new(
            format!(
              "`{name}` is missing required keyword `{}`",
              sig.param_names[i]
            ),
            expr.span,
          ));
        }
      }
      for (i, slot) in positional.iter().enumerate() {
        let Some(value) = slot else { continue };
        let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
        if actual != sig.params[i] {
          return Err(Diagnostic::new(
            format!(
              "keyword `{}` to `{name}` has type {actual:?}, expected {:?}",
              sig.param_names[i], sig.params[i]
            ),
            value.span,
          ));
        }
      }
      Ok(sig.return_type.clone())
    }
    Expr::New(class_name, args) => {
      let info = classes
        .get(class_name)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`"), expr.span))?;
      if info.is_module {
        return Err(Diagnostic::new(
          format!("cannot `.new` module `{class_name}` — modules are namespaces, not instantiable"),
          expr.span,
        ));
      }
      if info.is_actor {
        return Err(Diagnostic::new(
          format!(
            "cannot `.new` actor `{class_name}` — actors are constructed with `.spawn`, not `.new`"
          ),
          expr.span,
        ));
      }
      match info.methods.get("initialize") {
        Some(sig) => check_args(
          "initialize",
          args,
          &sig.params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?,
        None if args.is_empty() => {}
        None => {
          return Err(Diagnostic::new(
            format!(
              "`{class_name}.new` called with {} argument(s), but `{class_name}` declares no `initialize`",
              args.len()
            ),
            expr.span,
          ));
        }
      }
      Ok(Type::Class(class_name.clone()))
    }
    // Plan 54: `.spawn` is `.new`'s exact structural mirror — the same
    // `initialize`-arity/type check, the same inferred `Type::Class`
    // (an actor reference is typed and passed around identically to an
    // ordinary class instance in this plan's scope; only construction
    // and codegen's allocation call site differ) — but requires
    // `is_actor == true`, the exact opposite of `.new`'s two checks
    // above.
    Expr::Spawn(class_name, args) => {
      let info = classes
        .get(class_name)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`"), expr.span))?;
      if !info.is_actor {
        return Err(Diagnostic::new(
          format!(
            "cannot `.spawn` `{class_name}` — `.spawn` only constructs actors, and `{class_name}` is not one"
          ),
          expr.span,
        ));
      }
      match info.methods.get("initialize") {
        Some(sig) => check_args(
          "initialize",
          args,
          &sig.params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?,
        None if args.is_empty() => {}
        None => {
          return Err(Diagnostic::new(
            format!(
              "`{class_name}.spawn` called with {} argument(s), but `{class_name}` declares no `initialize`",
              args.len()
            ),
            expr.span,
          ));
        }
      }
      Ok(Type::Class(class_name.clone()))
    }
    // Plan 60's Decision log: `.spawn`'s distributed counterpart — the
    // identical `is_actor` gate, and the identical inferred type
    // (`Type::Class(class)`, Design decision 1: both a local and a
    // remote actor reference are the same tagged-handle shape at every
    // call site). `addr`/`name` must be `String`; no `initialize`
    // arity check here — `.remote` never constructs a new instance, it
    // resolves an already-`.register`ed one running in another process.
    Expr::Remote { class, addr, name } => {
      let info = classes
        .get(class)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class}`"), expr.span))?;
      if !info.is_actor {
        return Err(Diagnostic::new(
          format!(
            "cannot `.remote` `{class}` — `.remote` only resolves actors, and `{class}` is not one"
          ),
          expr.span,
        ));
      }
      let addr_ty = infer_expr_type(addr, env, sigs, classes, self_fields, gctx)?;
      if addr_ty != Type::String {
        return Err(Diagnostic::new(
          format!("`{class}.remote`'s address argument must be a String, found {addr_ty:?}"),
          addr.span,
        ));
      }
      let name_ty = infer_expr_type(name, env, sigs, classes, self_fields, gctx)?;
      if name_ty != Type::String {
        return Err(Diagnostic::new(
          format!("`{class}.remote`'s name argument must be a String, found {name_ty:?}"),
          name.span,
        ));
      }
      Ok(Type::Class(class.clone()))
    }
    // Plan 65's Decision log, `leaf-virtual-actor-placement`: `.locate`'s
    // own check — the identical `is_actor` gate `.spawn`/`.remote`
    // already use, `key` must be `String` (mirroring `.remote`'s own
    // `addr`/`name` checks), and `args` gets the identical `initialize`
    // arity/type check `.spawn` already performs (a `.locate` that
    // activates locally reuses `.spawn`'s own allocation path
    // verbatim — codegen's own `build_locate_call`). Inferred type is
    // `Type::Class(class)`, the same tagged-handle shape `.spawn`/
    // `.remote` already share — Design decision 1's own "every actor
    // reference looks identical at every call site" rule, extended to
    // a third construction form.
    Expr::Locate { class, key, args } => {
      let info = classes
        .get(class)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class}`"), expr.span))?;
      if !info.is_actor {
        return Err(Diagnostic::new(
          format!(
            "cannot `.locate` `{class}` — `.locate` only resolves/activates actors, and `{class}` is not one"
          ),
          expr.span,
        ));
      }
      let key_ty = infer_expr_type(key, env, sigs, classes, self_fields, gctx)?;
      if key_ty != Type::String {
        return Err(Diagnostic::new(
          format!("`{class}.locate`'s key argument must be a String, found {key_ty:?}"),
          key.span,
        ));
      }
      match info.methods.get("initialize") {
        Some(sig) => check_args(
          "initialize",
          args,
          &sig.params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?,
        None if args.is_empty() => {}
        None => {
          return Err(Diagnostic::new(
            format!(
              "`{class}.locate` called with {} argument(s), but `{class}` declares no `initialize`",
              args.len()
            ),
            expr.span,
          ));
        }
      }
      Ok(Type::Class(class.clone()))
    }
    // Plan 61's Decision log: `comptime`'s own legal *position*
    // restriction (a top-level `Let`'s direct value, `Array.new`'s
    // direct size argument) is enforced by a separate, standalone walk
    // — `check_comptime_positions`, invoked once from `check_program`,
    // the same "separate walk, not woven into the shared type-inference
    // plumbing" precedent `check_message_safety`/`check_block_call_
    // sites` already establish (`infer_expr_type` has no "am I at the
    // blessed position" context to thread without an invasive signature
    // change touching every call site). Type inference itself is
    // transparent: `comptime <expr>`'s type is simply `<expr>`'s own
    // type — the interpreter's job (`emerald-codegen`) is producing the
    // *value*, not changing the *type*.
    Expr::Comptime(inner) => infer_expr_type(inner, env, sigs, classes, self_fields, gctx),
    // Plan 57 (supervision trees), `leaf-supervise-declaration`:
    // `supervise do ... end`'s body is restricted to a flat list of
    // bound-or-bare `<Class>.spawn(<args>)` statements (Decision log —
    // any other shape, e.g. `puts "x"`, is a real diagnostic naming the
    // offending statement, not silently dropped and not a panic).
    // Every qualifying statement is type-checked by simply delegating
    // to `Expr::Spawn`'s own arm above (via `infer_expr_type` on its
    // `value`/the bare expression itself) — no duplicated arg-arity/
    // type-checking logic. A bound spawn's own declared annotation
    // must name the exact spawned class (`worker: Worker = Worker.
    // spawn(...)`, not e.g. `worker: Object = ...`) so codegen's own
    // `local_classes`-free, purely-syntactic body walk (`build_expr`'s
    // `Expr::Supervise` arm) never needs to consult a type at all.
    Expr::Supervise(body) => {
      let mut children: Vec<(Option<String>, Type)> = Vec::new();
      for stmt in body {
        match &stmt.node {
          Stmt::Let {
            name,
            ty,
            value:
              value @ Spanned {
                node: Expr::Spawn(class_name, _),
                ..
              },
            ..
          } => {
            if ty != class_name {
              return Err(Diagnostic::new(
                format!(
                  "`supervise do ... end`: `{name}: {ty} = {class_name}.spawn(...)` — declared type must match the spawned class `{class_name}`"
                ),
                stmt.span,
              ));
            }
            let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
            children.push((Some(name.clone()), actual));
          }
          Stmt::Expr(
            value @ Spanned {
              node: Expr::Spawn(_, _),
              ..
            },
          ) => {
            let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
            children.push((None, actual));
          }
          _ => {
            return Err(Diagnostic::new(
              "`supervise do ... end` body may only contain `<name>: <Class> = <Class>.spawn(<args>)` or bare `<Class>.spawn(<args>)` statements",
              stmt.span,
            ));
          }
        }
      }
      Ok(Type::Supervisor(children))
    }
    // Plan 45's Decision log: `File` reuses plan 12's `Name.method(args)`
    // dispatch *shape* but is a separate, hard-coded arm — `File` is
    // never declared via a real `ModuleDef`, so it never populates
    // `classes`/`is_module` and could never reach the module-dispatch
    // arm below regardless; checked first purely for arm-ordering
    // clarity, not to prevent an actual collision (verified: a program
    // is free to write `module File ... end`, which registers into
    // `classes` as normal — this arm never consults that registry at
    // all, so there is nothing to shadow).
    Expr::MethodCall(recv, method, args) if matches!(&recv.node, Expr::Ident(n) if n == "File") => {
      let (expected_params, ret) = match method.as_str() {
        "read" => (vec![Type::String], Type::String),
        "write" => (vec![Type::String, Type::String], Type::Void),
        other => {
          return Err(Diagnostic::new(
            format!("File has no method `{other}`"),
            expr.span,
          ));
        }
      };
      check_args(
        method,
        args,
        &expected_params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(ret)
    }
    // Plan 59's Decision log: `String.from_cstring(ptr)` — the same
    // reserved-namespace static-call shape as `File` immediately above,
    // for the same reason (`String` is never a real `ModuleDef`).
    // Plan 73's Decision log: `String?` (plan 43's now-removed `T?`)
    // becomes `Option[String]` — a C function's real `NULL` return is
    // exactly what `None` is for. `"Option$String"` is unconditionally
    // pre-instantiated by `check_program` (see its own Decision log)
    // specifically so this one compiler-internal use of `Option[String]`
    // — never spelled out as literal source text anywhere a program
    // might otherwise trigger its monomorphization — always exists.
    Expr::MethodCall(recv, method, args) if matches!(&recv.node, Expr::Ident(n) if n == "String") =>
    {
      if method != "from_cstring" {
        return Err(Diagnostic::new(
          format!("String has no static method `{method}`"),
          expr.span,
        ));
      }
      check_args(
        method,
        args,
        &[Type::CString],
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(Type::Enum("Option$String".to_string()))
    }
    // `Name.method(args)` on a module (plan 12) dispatches straight to
    // its method table — checked *before* the `.call`/`Type::Proc` arm
    // below (a module could in principle declare a method named `call`)
    // and before `infer_expr_type(recv)` runs at all, since a bare
    // module reference isn't a value — evaluating it as one would fail
    // with "undefined variable" (a module name is never in `env`).
    Expr::MethodCall(recv, method, args) if matches!(&recv.node, Expr::Ident(n) if classes.get(n).is_some_and(|c| c.is_module)) =>
    {
      let Expr::Ident(module_name) = &recv.node else {
        unreachable!()
      };
      let info = &classes[module_name];
      let sig = info.methods.get(method).ok_or_else(|| {
        Diagnostic::new(
          format!("module `{module_name}` has no method `{method}`"),
          expr.span,
        )
      })?;
      check_args(
        method,
        args,
        &sig.params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(sig.return_type.clone())
    }
    // Plan 41's Decision log: a method call on a `Type::Generic`-typed
    // receiver — a call inside the body of the generic function that
    // declares it, before this specific call site's own concrete type
    // is known — resolves against the bound interface's required
    // signature, with `Self` substituted with `Type::Generic` itself
    // (not the `classes` registry, which a `Type::Generic` never
    // appears in). Guarded syntactically (`env.get(n)`, not a value
    // this arm has already computed) since match-arm guards can't run
    // a fallible `infer_expr_type` call.
    Expr::MethodCall(recv, method, args) if matches!(&recv.node, Expr::Ident(n) if matches!(env.get(n), Some(Type::Generic(_, _)))) =>
    {
      let Expr::Ident(recv_name) = &recv.node else {
        unreachable!()
      };
      let Some(Type::Generic(type_param, bounds)) = env.get(recv_name) else {
        unreachable!()
      };
      // Plan 58's Decision log: a method call on an UNBOUNDED type
      // parameter (`class Box[T] ... end`'s own `T`, `bounds: []`) is
      // rejected outright here — there is no interface to resolve the
      // call against. Plan 88 widens `bounds` to a real conjunctive
      // set — the interface actually dispatched against is whichever
      // bound in the set declares a method of this name (a generic
      // function's own bound set is always exactly one entry today,
      // `check_generic_function_body`'s own construction, unchanged —
      // this only matters once a real multi-bound generic CLASS exists).
      if bounds.is_empty() {
        return Err(Diagnostic::new(
          format!(
            "cannot call a method on unbounded type parameter `{type_param}` — declare a bound, e.g. `Stack[T: Comparable]`, to call methods on values of type `{type_param}`"
          ),
          expr.span,
        ));
      };
      // Plan 89's Decision log: `InterfaceInfo` widened from a single
      // required method to a full `methods` list — this dispatch now
      // searches every bound interface's own method set by name,
      // instead of comparing directly against a single `method_name`.
      let iface_method = bounds
        .iter()
        .find_map(|b| {
          gctx
            .interfaces
            .get(b)
            .and_then(|iface| iface.methods.iter().find(|m| &m.method_name == method))
        })
        .ok_or_else(|| {
          Diagnostic::new(
            format!(
              "type parameter `{type_param}` (bounded by `{}`) has no method `{method}`",
              bounds.join(" + ")
            ),
            expr.span,
          )
        })?;
      let self_ty = Type::Generic(type_param.clone(), bounds.clone());
      let expected = iface_method
        .params_raw
        .iter()
        .map(|(_, raw)| {
          if raw.as_named() == Some("Self") {
            Ok(self_ty.clone())
          } else {
            resolve_type(raw, classes)
          }
        })
        .collect::<Result<Vec<_>, _>>()?;
      check_args(
        method,
        args,
        &expected,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if iface_method.return_type_raw.as_named() == Some("Self") {
        Ok(self_ty)
      } else {
        resolve_type(&iface_method.return_type_raw, classes)
      }
    }
    // Plan 57 (supervision trees): `.child(:name)` on a `Supervisor`-
    // typed receiver, mirroring `.call`'s own "dispatch is driven
    // purely by the value's own carried signature, not the `classes`
    // registry" pattern immediately below (`Type::Supervisor`'s own
    // doc comment: the tracked-children map travels with the value,
    // exactly like `Type::Proc`'s own signature) — ordered here, after
    // the module-dispatch/`Type::Generic` arms above, for the identical
    // reason `.call` is: a real module or generic-bounded receiver
    // declaring its own method literally named `child` must still
    // reach ITS OWN arm first.
    Expr::MethodCall(recv, method, args) if method == "child" => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      let Type::Supervisor(children) = &recv_ty else {
        return Err(Diagnostic::new(
          format!("method call `.child` on non-Supervisor type {recv_ty:?}"),
          recv.span,
        ));
      };
      let [arg] = &args[..] else {
        return Err(Diagnostic::new(
          format!(
            "`.child` expects exactly 1 argument (a symbol), found {}",
            args.len()
          ),
          expr.span,
        ));
      };
      let Expr::SymbolLit(child_name) = &arg.node else {
        return Err(Diagnostic::new(
          "`.child` requires a literal symbol argument (e.g. `sup.child(:worker)`)",
          arg.span,
        ));
      };
      children
        .iter()
        .find(|(n, _)| n.as_deref() == Some(child_name.as_str()))
        .map(|(_, t)| t.clone())
        .ok_or_else(|| {
          Diagnostic::new(
            format!("supervisor has no tracked child named `{child_name}`"),
            arg.span,
          )
        })
    }
    // `.call` on a `Proc`-typed receiver dispatches against the
    // signature carried directly on `Type::Proc` (plan 10) — everything
    // else falls through to the existing `Type::Class` method lookup.
    Expr::MethodCall(recv, method, args) if method == "call" => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      let Type::Proc(param_types, return_type) = &recv_ty else {
        return Err(Diagnostic::new(
          format!("method call `.call` on non-Proc type {recv_ty:?}"),
          recv.span,
        ));
      };
      check_args(
        "call",
        args,
        param_types,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok((**return_type).clone())
    }
    Expr::MethodCall(recv, method, args) => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      // Plan 73's Decision log: a direct `.method` on an `Option[T]`
      // receiver is a compile-time diagnostic naming the exact fix,
      // checked before the ordinary `Type::Class` match below (which
      // would otherwise reject it with the generic, less useful
      // "non-class type" message that arm already produces for other
      // mismatches) — the same "actual payoff" precedent plan 43's own
      // now-removed `T?` check established, retargeted at `Option[T]`.
      if is_option_type(&recv_ty, classes) {
        return Err(Diagnostic::new(
          format!(
            "method call `.{method}` on an `Option` receiver (type {recv_ty:?}) — use safe navigation `?.` or a `match` on `Some`/`None`"
          ),
          recv.span,
        ));
      }
      // Plan 42 (enumerable stdlib): `.key`/`.value` on a `Pair`-typed
      // receiver. Dispatched the identical way plan 45's `String` check
      // immediately below already is — by the receiver's own inferred
      // type, inside this shared fallthrough arm — and for the
      // identical reason (Decision log, re-confirmed as a REAL, not
      // hypothetical, bug this session: an earlier, name-guarded-arm
      // version of this exact check broke `examples/classes.em`'s own
      // pre-existing `Counter#value`/`Point#sum` methods by shadowing
      // them outright before ever reaching the real per-class method
      // table below).
      if let Type::Pair(k_ty, v_ty) = &recv_ty {
        if method != "key" && method != "value" {
          return Err(Diagnostic::new(
            format!("Pair has no method `{method}`"),
            expr.span,
          ));
        }
        if !args.is_empty() {
          return Err(Diagnostic::new(
            format!("`.{method}` takes no arguments, found {}", args.len()),
            expr.span,
          ));
        }
        return Ok(if method == "key" {
          (**k_ty).clone()
        } else {
          (**v_ty).clone()
        });
      }
      // Plan 42 (enumerable stdlib): `each`/`map`/`select`/`filter`/
      // `reduce`/`inject`/`each_with_index`/`count`/`sum`/`sort` on an
      // `Array[T]`/`Hash[K,V]`-typed receiver — real, disclosed
      // simplification from this plan's own literal design (see
      // `check_enumerable_call`'s own doc comment for the full "why a
      // hard-coded arm, not a real `Iterable[T]` interface" rationale).
      // Scoped to the receiver's own inferred type, same as `Pair`
      // immediately above and `String` immediately below — an
      // ARRAY/HASH-typed receiver can never collide with a real class's
      // own method table (no `ClassInfo` entry is ever `Type::Array`/
      // `Type::Hash`), so this check is sound without needing to try
      // the per-class lookup first at all; a receiver of any OTHER
      // type using one of these ten names (e.g. a real class's own
      // `.count`/`.each`) falls straight through, unaffected, to the
      // ordinary `Type::Class` dispatch below.
      if matches!(recv_ty, Type::Array(_) | Type::Hash(_, _))
        && matches!(
          method.as_str(),
          "each"
            | "map"
            | "select"
            | "filter"
            | "reduce"
            | "inject"
            | "each_with_index"
            | "count"
            | "sum"
            | "sort"
        )
      {
        return check_enumerable_call(
          &recv_ty,
          method,
          args,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
          expr.span,
        );
      }
      // Plan 45's Decision log: dispatched by checking the receiver's
      // *inferred type* here, inside the existing generic `MethodCall`
      // arm — not a new name-guarded arm, which would incorrectly
      // intercept a user-defined class method sharing a name with a
      // `String` intrinsic. Mirrors `puts`'s own "compiler intrinsic,
      // not an overloaded function" precedent.
      if recv_ty == Type::String {
        let Some((expected_params, ret)) = string_intrinsic_signature(method) else {
          return Err(Diagnostic::new(
            format!("String has no method `{method}`"),
            expr.span,
          ));
        };
        check_args(
          method,
          args,
          &expected_params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?;
        return Ok(ret);
      }
      let Type::Class(class_name) = &recv_ty else {
        return Err(Diagnostic::new(
          format!("method call `.{method}` on non-class type {recv_ty:?}"),
          recv.span,
        ));
      };
      let info = classes.get(class_name).ok_or_else(|| {
        Diagnostic::new(
          format!("internal error: unregistered class `{class_name}`"),
          expr.span,
        )
      })?;
      // Plan 60's Decision log: `recv.register(name, port)` — scoped by
      // the receiver's own ACTUAL type being an actor (mirroring the
      // `Array`/`Hash`/`String`/`Pair` intrinsic dispatches above, this
      // codebase's own established lesson: guard by receiver type, not
      // method name alone). A real, disclosed narrowing: an actor class
      // declaring its OWN method literally named `register` would be
      // shadowed here — the same accepted tradeoff `File`/`String`'s
      // own reserved-namespace dispatches already make for their names.
      if info.is_actor && method == "register" {
        check_args(
          "register",
          args,
          &[Type::String, Type::Int64],
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?;
        return Ok(Type::Void);
      }
      // Plan 88's Decision log: a generic method (`map[U](f: Proc[T,
      // U]): Array[U])`) is dispatched here, BEFORE the ordinary
      // `info.methods.get(method)` lookup below (a class can't declare
      // both an ordinary and a generic method of the same name — the
      // registration passes route a method into exactly one of
      // `methods`/`generic_methods`, never both). Infers the method's
      // own free type parameter by structurally unifying each declared
      // (already class-substituted) parameter type against the actual
      // argument's inferred `Type` (`infer_type_param_binding`),
      // requires every parameter that mentions it to agree on the same
      // binding (mirroring the top-level generic-function call site's
      // own identical consistency check above), then resolves the
      // REST of the signature — including a compound return type like
      // `Array[U]` — with that binding substituted in
      // (`resolve_with_type_param`).
      if let Some(g) = info.generic_methods.get(method) {
        let arg_types = args
          .iter()
          .map(|a| infer_expr_type(a, env, sigs, classes, self_fields, gctx))
          .collect::<Result<Vec<_>, _>>()?;
        if arg_types.len() != g.params_raw.len() {
          return Err(Diagnostic::new(
            format!(
              "`{method}` expects {} argument(s), found {}",
              g.params_raw.len(),
              arg_types.len()
            ),
            expr.span,
          ));
        }
        let mut concrete: Option<Type> = None;
        for (i, (_, raw)) in g.params_raw.iter().enumerate() {
          let Some(binding) = infer_type_param_binding(raw, &arg_types[i], &g.type_param) else {
            continue;
          };
          match &concrete {
            None => concrete = Some(binding),
            Some(c) if c != &binding => {
              return Err(Diagnostic::new(
                format!(
                  "type parameter `{}` resolved inconsistently in call to `{class_name}#{method}`: `{c:?}` at an earlier argument, `{binding:?}` at argument {}",
                  g.type_param,
                  i + 1
                ),
                args[i].span,
              ));
            }
            Some(_) => {}
          }
        }
        let concrete = concrete.ok_or_else(|| {
          Diagnostic::new(
            format!(
              "internal error: generic method `{class_name}#{method}` never uses its own type parameter `{}`",
              g.type_param
            ),
            expr.span,
          )
        })?;
        for bound in &g.bounds {
          let Type::Class(concrete_class) = &concrete else {
            return Err(Diagnostic::new(
              format!(
                "type parameter `{}` in call to `{class_name}#{method}` resolved to non-class type {concrete:?} — only a class implementing `{bound}` is a legal argument here",
                g.type_param
              ),
              expr.span,
            ));
          };
          let concrete_info = classes.get(concrete_class).ok_or_else(|| {
            Diagnostic::new(format!("undefined class `{concrete_class}`"), expr.span)
          })?;
          if concrete_info.implements.as_ref().map(|(n, _)| n.as_str()) != Some(bound.as_str()) {
            return Err(Diagnostic::new(
              format!(
                "`{concrete_class}` does not implement `{bound}`, required by generic method `{class_name}#{method}`'s type parameter `{}`",
                g.type_param
              ),
              expr.span,
            ));
          }
        }
        let effective_params = g
          .params_raw
          .iter()
          .map(|(_, raw)| resolve_with_type_param(raw, &g.type_param, &concrete, classes))
          .collect::<Result<Vec<_>, _>>()?;
        check_args(
          method,
          args,
          &effective_params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?;
        return resolve_with_type_param(&g.return_type_raw, &g.type_param, &concrete, classes);
      }
      let sig = info.methods.get(method).ok_or_else(|| {
        Diagnostic::new(
          format!("class `{class_name}` has no method `{method}`"),
          expr.span,
        )
      })?;
      check_args(
        method,
        args,
        &sig.params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      // Plan 65's `leaf-unified-fallible-send`: a cross-actor send's
      // real, compiled type moves from `Void` to `Result[Void,
      // SendError]` — every OTHER method call (an ordinary class's own
      // method, or an actor's own `self.method(...)` same-thread direct
      // call — codegen's own `build_method_call` Decision log states
      // this exact "literal `self` is always a direct call" rule, the
      // identical check reused here) keeps its ordinary declared return
      // type unchanged. `.register` (checked above, `Ok(Type::Void)`)
      // is a local-only setup call, never a cross-actor send, so it's
      // already excluded by construction (that branch already returned).
      if info.is_actor && !matches!(&recv.node, Expr::Ident(n) if n == "self") {
        return Ok(Type::Result(
          Box::new(Type::Void),
          Box::new(Type::Enum("SendError".to_string())),
        ));
      }
      Ok(sig.return_type.clone())
    }
    // Plan 73's Decision log: `?.` replaces plan 43's `&.` outright —
    // scoped to an `Option[T]`-typed receiver whose own `T` (looked up
    // from the SPECIFIC monomorphized enum's `Some` variant, not a
    // structural `Nullable(inner)` unwrap — `Option[T]` is a real ADT
    // now) is a class or `String`. Short-circuits to `None` at runtime
    // if the receiver was `None` (codegen's job); sema's job here is
    // just checking the call shape and computing the result type:
    // `Option[U]` where `U` is the dispatched method's own return type.
    // `Option[U]` must already be instantiated somewhere else in this
    // program (an explicit `x: Option[U] = ...` annotation, the same
    // "expected-type-providing position" every other construction-
    // ambiguity fix in this plan relies on) — `classes` is borrowed
    // immutably this deep in `infer_expr_type`, so a brand-new `U` this
    // program never otherwise names cannot be monomorphized on the fly
    // here; a real, disclosed limitation, not a silent wrong answer.
    Expr::SafeCall(recv, method, args) => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      if !is_option_type(&recv_ty, classes) {
        return Err(Diagnostic::new(
          format!("`?.{method}` requires an `Option[T]` receiver, found {recv_ty:?} — use `.` instead"),
          recv.span,
        ));
      }
      let Type::Enum(enum_name) = &recv_ty else {
        unreachable!("is_option_type only accepts Type::Enum");
      };
      let variants = classes[enum_name]
        .enum_variants
        .as_ref()
        .expect("Type::Enum is only ever constructed for a registered enum");
      let inner_ty = variants
        .iter()
        .find(|(n, _)| n == "Some")
        .and_then(|(_, fts)| fts.first())
        .cloned()
        .ok_or_else(|| {
          Diagnostic::new(
            format!("internal error: `{enum_name}` has no `Some` variant"),
            expr.span,
          )
        })?;
      let (ret_ty, expected_params) = match &inner_ty {
        Type::Class(class_name) => {
          let info = classes.get(class_name).ok_or_else(|| {
            Diagnostic::new(
              format!("internal error: unregistered class `{class_name}`"),
              expr.span,
            )
          })?;
          let sig = info.methods.get(method).ok_or_else(|| {
            Diagnostic::new(
              format!("class `{class_name}` has no method `{method}`"),
              expr.span,
            )
          })?;
          (sig.return_type.clone(), sig.params.clone())
        }
        Type::String => {
          let Some((expected_params, ret)) = string_intrinsic_signature(method) else {
            return Err(Diagnostic::new(
              format!("String has no method `{method}`"),
              expr.span,
            ));
          };
          (ret, expected_params)
        }
        other => {
          return Err(Diagnostic::new(
            format!(
              "`?.{method}` is only supported when `Option[T]`'s `T` is a class or String, found {other:?}"
            ),
            recv.span,
          ));
        }
      };
      check_args(
        method,
        args,
        &expected_params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      let Some(ret_annotation) = type_annotation_string(&ret_ty) else {
        return Err(Diagnostic::new(
          format!("`?.{method}` returns {ret_ty:?}, which cannot be re-wrapped in `Option[T]`"),
          expr.span,
        ));
      };
      let mangled = format!("Option${ret_annotation}");
      if !classes.contains_key(&mangled) {
        return Err(Diagnostic::new(
          format!(
            "`?.{method}` would produce `Option[{ret_annotation}]`, but that instantiation doesn't exist yet in this program — add an explicit `Option[{ret_annotation}]` type annotation somewhere (e.g. a `let`) so the compiler generates it"
          ),
          expr.span,
        ));
      }
      Ok(Type::Enum(mangled))
    }
    // Plan 73's Decision log: `lhs ?? rhs` — `lhs` must be `Option[T]`;
    // `rhs` must be the SAME `T` (no coercion, mirroring every other
    // "no implicit conversion" rule in this compiler). Purely a
    // desugaring onto the same match machinery `Option[T]` construction/
    // pattern-matching already uses (codegen's job) — sema's job is just
    // this type check.
    Expr::Coalesce(lhs, rhs) => {
      let lhs_ty = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
      if !is_option_type(&lhs_ty, classes) {
        return Err(Diagnostic::new(
          format!("`??`'s left operand must be `Option[T]`, found {lhs_ty:?}"),
          lhs.span,
        ));
      }
      let Type::Enum(enum_name) = &lhs_ty else {
        unreachable!("is_option_type only accepts Type::Enum");
      };
      let variants = classes[enum_name]
        .enum_variants
        .as_ref()
        .expect("Type::Enum is only ever constructed for a registered enum");
      let inner_ty = variants
        .iter()
        .find(|(n, _)| n == "Some")
        .and_then(|(_, fts)| fts.first())
        .cloned()
        .ok_or_else(|| {
          Diagnostic::new(
            format!("internal error: `{enum_name}` has no `Some` variant"),
            expr.span,
          )
        })?;
      let rhs_ty = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
      if rhs_ty != inner_ty {
        return Err(Diagnostic::new(
          format!(
            "`??`'s right operand has type {rhs_ty:?}, expected {inner_ty:?} (`Option[T]`'s own `T`)"
          ),
          rhs.span,
        ));
      }
      Ok(inner_ty)
    }
    Expr::InstanceVar(name) => {
      let fields = self_fields.ok_or_else(|| {
        Diagnostic::new(
          format!("`@{name}` used outside of a method body"),
          expr.span,
        )
      })?;
      fields
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined field `@{name}`"), expr.span))
    }
    // Empty arrays are rejected (plan 09's Decision log): with no
    // structured type annotation on the literal itself, an empty
    // `[]` has no element type to infer — `Array[T]`'s declared `T`
    // on the enclosing `Let` isn't visible from here.
    Expr::ArrayLit(elements) => {
      infer_array_lit_type(elements, env, sigs, classes, self_fields, gctx)
    }
    // Plan 25: `Hash`'s get/set reuse `Expr::Index`/`Stmt::SetIndex`
    // (see the Decision log) — a `Type::Hash(_, _)` arm sits alongside
    // the existing `Type::Array(_)` one rather than a parallel indexing
    // mechanism.
    Expr::Index(array, index) => {
      let array_ty = infer_expr_type(array, env, sigs, classes, self_fields, gctx)?;
      let index_ty = infer_expr_type(index, env, sigs, classes, self_fields, gctx)?;
      match array_ty {
        Type::Array(elem_ty) => {
          if index_ty != Type::Int64 {
            return Err(Diagnostic::new(
              format!("array index must be Int64, found {index_ty:?}"),
              index.span,
            ));
          }
          Ok(*elem_ty)
        }
        Type::Hash(key_ty, value_ty) => {
          if index_ty != *key_ty {
            return Err(Diagnostic::new(
              format!("Hash key must be {key_ty:?}, found {index_ty:?}"),
              index.span,
            ));
          }
          Ok(*value_ty)
        }
        // Plan 40's Decision log: the third arm of this now-three-way
        // match — a class declaring a one-parameter `"[]"` method.
        Type::Class(class_name) => resolve_class_operator(
          "[]",
          &class_name,
          index,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        ),
        // Plan 45's Decision log: `str[i]` — a real one-character
        // `String`, not an `Int64` byte value.
        Type::String => {
          if index_ty != Type::Int64 {
            return Err(Diagnostic::new(
              format!("String index must be Int64, found {index_ty:?}"),
              index.span,
            ));
          }
          Ok(Type::String)
        }
        other => Err(Diagnostic::new(
          format!("`[...]` indexing requires an Array or a Hash, found {other:?}"),
          array.span,
        )),
      }
    }
    Expr::Lambda {
      params,
      return_type,
      body,
    } => infer_lambda_type(params, return_type, body, env, sigs, classes, gctx),
    // Plan 25: a real `Boolean` value, not just `Compare`'s byproduct.
    Expr::Bool(_) => Ok(Type::Boolean),
    // A `{}` empty literal has no key/value type to infer — same
    // reasoning `infer_array_lit_type` already applies to `[]` (plan
    // 09's Decision log), applied here for the second container kind.
    Expr::HashLit(pairs) => infer_hash_lit_type(pairs, env, sigs, classes, self_fields, gctx),
    Expr::ArrayNew(size) => {
      let size_ty = infer_expr_type(size, env, sigs, classes, self_fields, gctx)?;
      if size_ty != Type::Int64 {
        return Err(Diagnostic::new(
          format!("`Array.new` size must be Int64, found {size_ty:?}"),
          size.span,
        ));
      }
      // `Array.new(size)`'s element type comes from the enclosing
      // `Let`'s declared annotation (plan 25's Decision log — the same
      // source plan 09 already uses for an array literal's element
      // type) — `check_stmt`'s `Let` case special-cases this the same
      // way it already special-cases `Proc`, since `infer_expr_type`
      // alone has no declared-type context to draw on here.
      Err(Diagnostic::new(
        "`Array.new(...)` may only appear as a top-level `Let`'s value, where its element type is known from the declared annotation",
        expr.span,
      ))
    }
    // Plan 39's Decision log: the grammar only ever constructs this
    // node from `return a, b`'s comma-list `Stmt::Return` rule, so
    // there's no separate "TupleLit used somewhere illegal" case to
    // reject here — whatever `Type::Tuple` this produces either
    // matches the enclosing function's declared tuple return type (via
    // `Stmt::Return`'s existing `is_assignable` check, unmodified) or
    // fails as an ordinary type mismatch, the same as any other
    // wrong-shaped return value.
    Expr::TupleLit(elems) => {
      let types = elems
        .iter()
        .map(|e| infer_expr_type(e, env, sigs, classes, self_fields, gctx))
        .collect::<Result<Vec<_>, _>>()?;
      Ok(Type::Tuple(types))
    }
    // Plan 53's Decision log: `Ok`/`Err` are checked only in the three
    // expected-type-providing positions (`Let`'s declared type,
    // `Assign`'s recorded type, `Return`'s threaded return type) —
    // `check_stmt` special-cases all three *before* ever calling
    // `infer_expr_type` on one of these nodes directly. Reached here at
    // all means neither of those special cases fired — a real,
    // disclosed diagnostic, not a silent best-effort guess, since
    // `infer_expr_type` has no expected-type parameter to resolve
    // `Result[T, E]`'s type parameters from.
    Expr::Ok(_) | Expr::Err(_) => Err(Diagnostic::new(
      "cannot infer `Result[T, E]`'s type parameters without a declared expected type here — `Ok`/`Err` are only supported directly as a `let`, assignment, or `return` value",
      expr.span,
    )),
    // Plan 53's Decision log: `?` is restricted to exactly two
    // syntactic positions (a `Stmt::Let`'s or `Stmt::Assign`'s direct
    // value) — `check_stmt` special-cases both *before* ever calling
    // `infer_expr_type` on a `Try` node directly. Reached here at all
    // means `?` was used somewhere else (nested inside a binary
    // operator, as a bare call argument, as a `Return`'s value, etc.).
    Expr::Try(_) => Err(Diagnostic::new(
      "`?` is only supported directly as a `let` or assignment value in v1",
      expr.span,
    )),
  }
}

/// All key/value pairs of a hash literal must share one key type and
/// one value type (independently — a mixed-key-type *or*
/// mixed-value-type literal is rejected), and — since there's no
/// structured type annotation on the literal itself to fall back on —
/// the literal can't be empty (mirrors `infer_array_lit_type` exactly).
fn infer_hash_lit_type(
  pairs: &[(Spanned<Expr>, Spanned<Expr>)],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let Some(((first_k, first_v), rest)) = pairs.split_first() else {
    // No pair at all to blame for a real span — an honest (0, 0), see
    // `Diagnostic::span`'s own doc comment.
    return Err(Diagnostic::new(
      "empty hash literals are not supported — the key/value types can't be inferred",
      (0, 0),
    ));
  };
  let key_ty = infer_expr_type(first_k, env, sigs, classes, self_fields, gctx)?;
  let value_ty = infer_expr_type(first_v, env, sigs, classes, self_fields, gctx)?;
  for (i, (k, v)) in rest.iter().enumerate() {
    let kt = infer_expr_type(k, env, sigs, classes, self_fields, gctx)?;
    if kt != key_ty {
      return Err(Diagnostic::new(
        format!(
          "hash literal pair {} has key type {kt:?}, expected {key_ty:?} (all keys must share one type)",
          i + 2
        ),
        k.span,
      ));
    }
    let vt = infer_expr_type(v, env, sigs, classes, self_fields, gctx)?;
    if vt != value_ty {
      return Err(Diagnostic::new(
        format!(
          "hash literal pair {} has value type {vt:?}, expected {value_ty:?} (all values must share one type)",
          i + 2
        ),
        v.span,
      ));
    }
  }
  Ok(Type::Hash(Box::new(key_ty), Box::new(value_ty)))
}

/// A lambda body is checked exactly like a function body — the outer
/// scope's locals plus the lambda's own params, no `@field` access (`None`
/// self_fields; plan 10's Decision log restricts lambdas to top-level
/// `Let`s, where there's no enclosing method anyway).
///
/// Plan 71's Decision log: after the grammar unification, a lambda
/// literal's surface syntax (`do |params: T| ... end`) no longer states
/// a return type at all — `return_type` here is always the `"Void"`
/// placeholder the parser fills in (see `Expr::Lambda`'s own doc
/// comment), never a real user-written annotation. Unlike an ordinary
/// function/method (whose `return_type` is mandatory, real, surface
/// syntax and is *checked against*), a lambda's return type is instead
/// *inferred* from its own body — the type of the body's trailing
/// implicit-return expression (or of an explicit `return <expr>` in
/// that same trailing position), falling back to `Void` for an empty
/// body or one that doesn't end in either shape. This closes the
/// disclosed gap plan 71 itself left open: every one of this crate's
/// existing Proc-consuming call sites (`.select`/`.map`/`.reduce`/
/// `.count`/generic-function binding/etc.) reads the real signature
/// back out of `env` (see `check_stmt`'s own `Proc`-typed `Let` case),
/// so a bogus `Void` here previously broke every one of them, not just
/// a body/return-type mismatch on the lambda itself.
fn infer_lambda_type(
  params: &[Param],
  _return_type: &TypeExpr,
  body: &[Spanned<Stmt>],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let mut lambda_env = env.clone();
  let mut param_types = Vec::with_capacity(params.len());
  for p in params {
    let t = resolve_type(&p.ty, classes)?;
    param_types.push(t.clone());
    lambda_env.insert(p.name.clone(), t);
  }
  let inferred_return = match body.last() {
    None => Type::Void,
    Some(Spanned {
      node: Stmt::Expr(e),
      ..
    })
    | Some(Spanned {
      node: Stmt::Return(Some(e)),
      ..
    }) => infer_expr_type(e, &lambda_env, sigs, classes, None, gctx)?,
    Some(_) => Type::Void,
  };
  // Plan 34: `yields_allowed = false` — a lambda/block literal's own
  // body is never itself a `yield`-legal context (only a function/
  // method that declares `block_param` is), whether this is plan 10's
  // original top-level `Proc` lambda or plan 34's block literal reusing
  // the same `Expr::Lambda` node.
  // Plan 72's Decision log: a lambda/block-literal parameter is
  // immutable by construction, the same as an ordinary function's — a
  // fresh, empty set (no outer `mutable_locals` to inherit: a lambda
  // body's own scope for this purpose starts clean, matching
  // `lambda_env`'s own fresh-`HashMap` precedent immediately above).
  let mut lambda_mutable: HashSet<String> = HashSet::new();
  check_block(
    body,
    &mut lambda_env,
    &mut lambda_mutable,
    sigs,
    classes,
    None,
    &inferred_return,
    false,
    false,
    false,
    gctx,
  )?;
  check_implicit_return(
    body,
    &lambda_env,
    sigs,
    classes,
    None,
    &inferred_return,
    "<lambda>",
    gctx,
  )?;
  Ok(Type::Proc(param_types, Box::new(inferred_return)))
}

/// All elements of an array literal must share one type, and — since
/// there's no structured type annotation on the literal itself to fall
/// back on — the literal can't be empty (plan 09's Decision log).
fn infer_array_lit_type(
  elements: &[Spanned<Expr>],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let Some((first, rest)) = elements.split_first() else {
    return Err(Diagnostic::new(
      "empty array literals are not supported — the element type can't be inferred",
      (0, 0),
    ));
  };
  let elem_ty = infer_expr_type(first, env, sigs, classes, self_fields, gctx)?;
  for (i, e) in rest.iter().enumerate() {
    let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
    if t != elem_ty {
      return Err(Diagnostic::new(
        format!(
          "array literal element {} has type {t:?}, expected {elem_ty:?} (all elements must share one type)",
          i + 2
        ),
        e.span,
      ));
    }
  }
  Ok(Type::Array(Box::new(elem_ty)))
}

#[allow(clippy::too_many_arguments)]
fn check_args(
  name: &str,
  args: &[Spanned<Expr>],
  expected: &[Type],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  if args.len() != expected.len() {
    // Arity is a property of the whole call, not any one argument
    // (Decision log) — the honest fallback is the first arg's span
    // when one exists (closest real position to "the call"), or
    // `(0, 0)` for a zero-arg call with too many expected.
    let span = args.first().map(|a| a.span).unwrap_or((0, 0));
    return Err(Diagnostic::new(
      format!(
        "`{name}` expects {} argument(s), found {}",
        expected.len(),
        args.len()
      ),
      span,
    ));
  }
  for (i, (arg, expected_ty)) in args.iter().zip(expected).enumerate() {
    let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
    if !is_assignable(&actual, expected_ty) {
      return Err(Diagnostic::new(
        format!(
          "argument {} to `{name}` has type {actual:?}, expected {expected_ty:?}",
          i + 1
        ),
        arg.span,
      ));
    }
  }
  Ok(())
}

/// Plan 39: `Expr::Call`'s own arity/type checking — unlike `check_args`
/// above (reused verbatim by `Expr::New`/module-method calls, which get
/// none of this plan's four features, per its Decision log's method/
/// non-function scoping), this accepts fewer than `sig.params.len()`
/// arguments as long as every missing trailing one has a declared
/// default, and — when `sig.splat_elem` is `Some` — accepts any number
/// of trailing arguments beyond `sig.params.len()`, type-checked
/// against the splat's declared element type.
#[allow(clippy::too_many_arguments)]
fn check_call_args(
  name: &str,
  args: &[Spanned<Expr>],
  sig: &FunctionSig,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  // Plan 22's own concrete AC2: an arity diagnostic's span is the
  // *whole call expression* (`add(20)`), not any one argument's own
  // span — arity is a property of the call, never of an individual
  // argument (Decision log).
  call_span: (usize, usize),
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let required = sig.params.len();
  if sig.splat_elem.is_none() && args.len() > required {
    return Err(Diagnostic::new(
      format!(
        "`{name}` expects {required} argument(s), found {}",
        args.len()
      ),
      call_span,
    ));
  }
  if args.len() < required {
    for (i, default) in sig.defaults.iter().enumerate().skip(args.len()) {
      if default.is_none() {
        return Err(Diagnostic::new(
          format!(
            "`{name}` is missing required argument `{}`",
            sig.param_names[i]
          ),
          call_span,
        ));
      }
    }
  }
  let checked = args.len().min(required);
  for (i, arg) in args.iter().enumerate().take(checked) {
    let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
    if actual != sig.params[i] {
      return Err(Diagnostic::new(
        format!(
          "argument {} to `{name}` has type {actual:?}, expected {:?}",
          i + 1,
          sig.params[i]
        ),
        arg.span,
      ));
    }
  }
  if let Some(splat_ty) = &sig.splat_elem {
    for (i, arg) in args.iter().enumerate().skip(required) {
      let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
      if actual != *splat_ty {
        return Err(Diagnostic::new(
          format!(
            "trailing (splat) argument {} to `{name}` has type {actual:?}, expected {splat_ty:?}",
            i + 1
          ),
          arg.span,
        ));
      }
    }
  }
  check_requires_static_provability(name, args, sig, call_span)?;
  Ok(())
}

/// Plan 62's `leaf-sema-static-provability`: catches exactly one narrow
/// case — a `requires` clause whose every free parameter-identifier is
/// bound, at THIS call site, to a bare literal argument, folding to
/// `Some(false)`. Wired only into `check_call_args` (`Expr::Call`'s own
/// checker) — never `check_args` (`Expr::New`/module-method calls), per
/// the Decision log's explicit method/non-function scoping.
fn check_requires_static_provability(
  name: &str,
  args: &[Spanned<Expr>],
  sig: &FunctionSig,
  call_span: (usize, usize),
) -> Result<(), Diagnostic> {
  if sig.requires.is_empty() {
    return Ok(());
  }
  let bindings: HashMap<&str, &Expr> = sig
    .param_names
    .iter()
    .zip(args.iter())
    .filter(|(_, a)| {
      matches!(
        &a.node,
        Expr::Int(_) | Expr::Float(_) | Expr::StringLit(_) | Expr::Bool(_)
      )
    })
    .map(|(pname, a)| (pname.as_str(), &a.node))
    .collect();
  if bindings.is_empty() {
    return Ok(());
  }
  for c in &sig.requires {
    if eval_const_bool(&c.expr.node, &bindings) == Some(false) {
      let mut used: Vec<&str> = Vec::new();
      collect_const_bool_idents(&c.expr.node, &mut used);
      let literal_args = used
        .into_iter()
        .filter_map(|n| {
          bindings
            .get(n)
            .map(|e| format!("{n} = {}", format_literal_expr(e)))
        })
        .collect::<Vec<_>>()
        .join(", ");
      return Err(Diagnostic::new(
        format!(
          "contract violation provable at compile time: `{name}`'s requires `{}` is false for the literal arguments given at this call site ({literal_args})",
          c.text
        ),
        call_span,
      ));
    }
  }
  Ok(())
}

fn format_literal_expr(e: &Expr) -> String {
  match e {
    Expr::Int(n) => n.to_string(),
    Expr::Float(f) => f.to_string(),
    Expr::StringLit(s) => format!("{s:?}"),
    Expr::Bool(b) => b.to_string(),
    _ => "?".to_string(),
  }
}

/// Free identifiers reachable through exactly `eval_const_bool`'s own
/// closed grammar — used only to name which literal-bound parameters a
/// failed clause actually referenced in its own violation message
/// (`divide`'s worked example: `requires b != 0` names only `b`, not
/// `a`, even when both happen to be literal at the call site).
fn collect_const_bool_idents<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
  match expr {
    Expr::Ident(name) => out.push(name.as_str()),
    Expr::Int(_) | Expr::Float(_) | Expr::StringLit(_) | Expr::Bool(_) => {}
    Expr::Neg(a) | Expr::Not(a) => collect_const_bool_idents(&a.node, out),
    Expr::And(a, b)
    | Expr::Or(a, b)
    | Expr::Add(a, b)
    | Expr::Sub(a, b)
    | Expr::Mul(a, b)
    | Expr::Div(a, b)
    | Expr::Rem(a, b) => {
      collect_const_bool_idents(&a.node, out);
      collect_const_bool_idents(&b.node, out);
    }
    Expr::Compare(a, _, b) => {
      collect_const_bool_idents(&a.node, out);
      collect_const_bool_idents(&b.node, out);
    }
    _ => {}
  }
}

/// Plan 62's `leaf-sema-static-provability`: a small, new, purpose-built
/// evaluator — verified this session that no general constant-folding
/// mechanism exists anywhere in `emerald-sema`/`emerald-codegen` today
/// (see the Decision log). Returns `None` the instant it hits anything
/// outside this closed grammar (an `Ident` not present in `bindings`, a
/// method/field/index access, etc.) — never a panic, never a guessed
/// answer.
#[derive(Debug, Clone, PartialEq)]
enum ConstLit {
  Int(i64),
  Float(f64),
  Str(String),
  Bool(bool),
}

fn eval_const_lit(expr: &Expr, bindings: &HashMap<&str, &Expr>) -> Option<ConstLit> {
  match expr {
    Expr::Int(n) => Some(ConstLit::Int(*n)),
    Expr::Float(f) => Some(ConstLit::Float(*f)),
    Expr::StringLit(s) => Some(ConstLit::Str(s.clone())),
    Expr::Bool(b) => Some(ConstLit::Bool(*b)),
    Expr::Ident(name) => eval_const_lit(bindings.get(name.as_str())?, bindings),
    Expr::Neg(a) => match eval_const_lit(&a.node, bindings)? {
      ConstLit::Int(x) => Some(ConstLit::Int(-x)),
      ConstLit::Float(x) => Some(ConstLit::Float(-x)),
      _ => None,
    },
    Expr::Not(a) => match eval_const_lit(&a.node, bindings)? {
      ConstLit::Bool(b) => Some(ConstLit::Bool(!b)),
      _ => None,
    },
    Expr::And(a, b) => match (
      eval_const_lit(&a.node, bindings)?,
      eval_const_lit(&b.node, bindings)?,
    ) {
      (ConstLit::Bool(x), ConstLit::Bool(y)) => Some(ConstLit::Bool(x && y)),
      _ => None,
    },
    Expr::Or(a, b) => match (
      eval_const_lit(&a.node, bindings)?,
      eval_const_lit(&b.node, bindings)?,
    ) {
      (ConstLit::Bool(x), ConstLit::Bool(y)) => Some(ConstLit::Bool(x || y)),
      _ => None,
    },
    Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) | Expr::Rem(a, b) => {
      let av = eval_const_lit(&a.node, bindings)?;
      let bv = eval_const_lit(&b.node, bindings)?;
      eval_const_arith(expr, av, bv)
    }
    Expr::Compare(a, op, b) => {
      let av = eval_const_lit(&a.node, bindings)?;
      let bv = eval_const_lit(&b.node, bindings)?;
      eval_const_compare(*op, &av, &bv)
    }
    _ => None,
  }
}

fn eval_const_arith(expr: &Expr, a: ConstLit, b: ConstLit) -> Option<ConstLit> {
  match (a, b) {
    (ConstLit::Int(x), ConstLit::Int(y)) => Some(ConstLit::Int(match expr {
      Expr::Add(..) => x.checked_add(y)?,
      Expr::Sub(..) => x.checked_sub(y)?,
      Expr::Mul(..) => x.checked_mul(y)?,
      Expr::Div(..) if y != 0 => x.checked_div(y)?,
      Expr::Rem(..) if y != 0 => x.checked_rem(y)?,
      _ => return None,
    })),
    (ConstLit::Float(x), ConstLit::Float(y)) => Some(ConstLit::Float(match expr {
      Expr::Add(..) => x + y,
      Expr::Sub(..) => x - y,
      Expr::Mul(..) => x * y,
      Expr::Div(..) => x / y,
      Expr::Rem(..) => x % y,
      _ => return None,
    })),
    _ => None,
  }
}

fn eval_const_compare(op: CompareOp, a: &ConstLit, b: &ConstLit) -> Option<ConstLit> {
  let ordering = match (a, b) {
    (ConstLit::Int(x), ConstLit::Int(y)) => x.partial_cmp(y),
    (ConstLit::Float(x), ConstLit::Float(y)) => x.partial_cmp(y),
    (ConstLit::Str(x), ConstLit::Str(y)) => x.partial_cmp(y),
    (ConstLit::Bool(x), ConstLit::Bool(y)) if matches!(op, CompareOp::Eq | CompareOp::Ne) => {
      x.partial_cmp(y)
    }
    _ => return None,
  }?;
  Some(ConstLit::Bool(match op {
    CompareOp::Lt => ordering.is_lt(),
    CompareOp::Gt => ordering.is_gt(),
    CompareOp::Le => ordering.is_le(),
    CompareOp::Ge => ordering.is_ge(),
    CompareOp::Eq => ordering.is_eq(),
    CompareOp::Ne => ordering.is_ne(),
  }))
}

fn eval_const_bool(expr: &Expr, bindings: &HashMap<&str, &Expr>) -> Option<bool> {
  match eval_const_lit(expr, bindings)? {
    ConstLit::Bool(b) => Some(b),
    _ => None,
  }
}

/// `arr[i] = value` — array element must be Int64-indexed and the RHS
/// must match the array's element type.
#[allow(clippy::too_many_arguments)]
fn check_set_index(
  array: &Spanned<Expr>,
  index: &Spanned<Expr>,
  value: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let array_ty = infer_expr_type(array, env, sigs, classes, self_fields, gctx)?;
  let index_ty = infer_expr_type(index, env, sigs, classes, self_fields, gctx)?;
  let (container, elem_ty, index_expected) = match array_ty {
    Type::Array(elem_ty) => ("array", *elem_ty, Type::Int64),
    Type::Hash(key_ty, value_ty) => ("Hash", *value_ty, *key_ty),
    // Plan 40's Decision log: sourced from a two-parameter `"[]="`
    // method's own declared signature (`(index, value) -> Void`) — the
    // rest of this function's index-type/value-type checking below is
    // reused completely unchanged for this third case.
    Type::Class(class_name) => {
      let info = classes
        .get(&class_name)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`"), array.span))?;
      let sig = info.methods.get("[]=").ok_or_else(|| {
        Diagnostic::new(
          format!("class `{class_name}` has no operator method `[]=`"),
          array.span,
        )
      })?;
      if sig.params.len() != 2 {
        return Err(Diagnostic::new(
          format!(
            "class `{class_name}`'s `[]=` method must take exactly 2 parameters (index, value), found {}",
            sig.params.len()
          ),
          array.span,
        ));
      }
      ("[]=", sig.params[1].clone(), sig.params[0].clone())
    }
    other => {
      return Err(Diagnostic::new(
        format!("`[...] = ...` indexing requires an Array or a Hash, found {other:?}"),
        array.span,
      ));
    }
  };
  if index_ty != index_expected {
    return Err(Diagnostic::new(
      format!("{container} index must be {index_expected:?}, found {index_ty:?}"),
      index.span,
    ));
  }
  let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
  if !is_assignable(&actual, &elem_ty) {
    return Err(Diagnostic::new(
      format!(
        "type mismatch in {container} assignment: element type is {elem_ty:?}, value has type {actual:?}"
      ),
      value.span,
    ));
  }
  Ok(())
}

/// Plan 39's Decision log: `x, y = f()`'s own arity/type check against
/// the tuple `f` actually returned — separated out of `check_multi_
/// assign` to keep each path's own complexity down.
fn check_tuple_multi_assign(
  names: &[String],
  value: &Spanned<Expr>,
  ts: &[Type],
  env: &HashMap<String, Type>,
  mutable_locals: &HashSet<String>,
) -> Result<(), Diagnostic> {
  if ts.len() != names.len() {
    return Err(Diagnostic::new(
      format!(
        "multiple assignment arity mismatch: {} target(s), {}-element tuple returned",
        names.len(),
        ts.len()
      ),
      value.span,
    ));
  }
  for (i, (name, t)) in names.iter().zip(ts.iter()).enumerate() {
    let declared = env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), value.span))?;
    check_mutable(name, mutable_locals, value.span)?;
    if !is_assignable(t, &declared) {
      return Err(Diagnostic::new(
        format!(
          "type mismatch in multiple assignment at position {}: `{name}` has type {declared:?}, tuple element has type {t:?}",
          i + 1
        ),
        value.span,
      ));
    }
  }
  Ok(())
}

/// `n1, n2, ... = v1, v2, ...` (plan 31's Decision log): fixed-arity
/// only — an arity mismatch is a real diagnostic, not a panic or silent
/// truncation/padding. Every `values` expression is type-checked before
/// any `names` binding is consulted for its target type, matching
/// codegen's own "evaluate all RHS before writing any target" ordering
/// (what makes `a, b = b, a` a real swap).
#[allow(clippy::too_many_arguments)]
fn check_multi_assign(
  names: &[String],
  values: &[Spanned<Expr>],
  env: &HashMap<String, Type>,
  mutable_locals: &HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
  stmt_span: (usize, usize),
) -> Result<(), Diagnostic> {
  // Plan 72's Decision log: two separate passes over `names`, not one
  // combined loop — every target's EXISTENCE is confirmed first (an
  // undefined-variable diagnostic always takes priority, matching
  // `Stmt::Assign`'s own ordering), and only once every target is
  // confirmed to exist does the second pass check `mutable_locals`.
  // Interleaving the two into one loop would make an earlier
  // immutable-but-declared name (`a` in `a, z = 1, 2` when `z` is
  // undefined) shadow a later target's genuine "undefined variable"
  // diagnostic, purely because of its position in `names` — an
  // accidental, position-dependent behavior, not a real design choice.
  for name in names {
    if !env.contains_key(name) {
      return Err(Diagnostic::new(
        format!("undefined variable `{name}`"),
        stmt_span,
      ));
    }
  }
  for name in names {
    check_mutable(name, mutable_locals, stmt_span)?;
  }
  // Plan 39's Decision log: `x, y = f()` — a single call-shaped value
  // whose declared return type is a `Type::Tuple` of matching arity —
  // unpacks positionally, entirely ahead of the ordinary per-value path
  // below, which never anticipated a single value expression producing
  // more than one result. Every pre-existing shape (`values.len() ==
  // names.len()`, no call involved, or a call whose return type isn't
  // a tuple) falls straight through to that unmodified path — this is
  // a genuinely additive special case, not a rewrite.
  if let [value] = values {
    if let Expr::Call(..) = &value.node {
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if let Type::Tuple(ts) = &actual {
        return check_tuple_multi_assign(names, value, ts, env, mutable_locals);
      }
    }
  }
  if names.len() != values.len() {
    let span = values.first().map(|v| v.span).unwrap_or((0, 0));
    return Err(Diagnostic::new(
      format!(
        "multiple assignment arity mismatch: {} target(s), {} value(s)",
        names.len(),
        values.len()
      ),
      span,
    ));
  }
  let value_types = values
    .iter()
    .map(|v| infer_expr_type(v, env, sigs, classes, self_fields, gctx))
    .collect::<Result<Vec<_>, _>>()?;
  for (i, (name, actual)) in names.iter().zip(value_types).enumerate() {
    let value_span = values[i].span;
    let declared = env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), value_span))?;
    if !is_assignable(&actual, &declared) {
      return Err(Diagnostic::new(
        format!(
          "type mismatch in multiple assignment at position {}: `{name}` has type {declared:?}, value has type {actual:?}",
          i + 1
        ),
        value_span,
      ));
    }
  }
  Ok(())
}

/// Plan 53's Decision log: `Ok`/`Err` construction, checked only where
/// an expected `Result[T, E]` type is already on hand — this helper is
/// shared by all three call sites (`Let`, `Assign`, `Return`) rather
/// than duplicated three times.
fn check_result_construction(
  declared: &Type,
  value: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let Type::Result(t_ty, e_ty) = declared else {
    return Err(Diagnostic::new(
      format!(
        "`Ok`/`Err` construction requires a declared `Result[T, E]` type, found {declared:?}"
      ),
      value.span,
    ));
  };
  match &value.node {
    Expr::Ok(inner) => {
      let actual = infer_expr_type(inner, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, t_ty) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `Ok(...)`: declared `T` is {t_ty:?}, value has type {actual:?}"
          ),
          inner.span,
        ));
      }
      Ok(())
    }
    Expr::Err(inner) => {
      let actual = infer_expr_type(inner, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, e_ty) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `Err(...)`: declared `E` is {e_ty:?}, value has type {actual:?}"
          ),
          inner.span,
        ));
      }
      Ok(())
    }
    _ => unreachable!("caller only invokes this for Expr::Ok/Expr::Err"),
  }
}

/// Plan 53's Decision log: `?`'s legality/exact-`E`-match check, reusing
/// the already-threaded `return_type` — no new parameter added to
/// `check_stmt`'s own signature. Returns the unwrapped `T`; the caller
/// (`Let`/`Assign`) still checks that against its own target type,
/// exactly like every other value-producing expression already does.
fn check_try(
  inner: &Spanned<Expr>,
  return_type: &Type,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let Type::Result(_, ret_e_ty) = return_type else {
    return Err(Diagnostic::new(
      "`?` may only be used inside a function whose own declared return type is `Result[T, E]`",
      inner.span,
    ));
  };
  let inner_ty = infer_expr_type(inner, env, sigs, classes, self_fields, gctx)?;
  let Type::Result(t_ty, e_ty) = &inner_ty else {
    return Err(Diagnostic::new(
      format!("`?` requires a `Result[T, E]`-typed expression, found {inner_ty:?}"),
      inner.span,
    ));
  };
  if e_ty.as_ref() != ret_e_ty.as_ref() {
    return Err(Diagnostic::new(
      format!(
        "`?`'s error type {e_ty:?} does not match the enclosing function's declared error type {ret_e_ty:?} — no automatic conversion"
      ),
      inner.span,
    ));
  }
  Ok((**t_ty).clone())
}

/// Plan 72's Decision log: records a fresh (or re-declared/shadowed)
/// local's mutability alongside its type in the same motion as binding
/// it — `is_var: true` on some `Stmt::Let` is the ONLY way a name ever
/// enters `mutable_locals`; every other binding-introducing construct
/// in this file (a function/method parameter, a splat parameter, a
/// `for`/`for...in..` induction variable, a `case`/`match`-arm
/// pattern binding, a `rescue` clause's own exception variable) calls
/// `env.insert` directly instead of this helper, so none of them are
/// ever added to `mutable_locals` and stay immutable by construction —
/// a deliberate, uniform default rather than a per-construct decision.
/// Re-declaring the same name via a second `Let` always resets its
/// mutability to match the new declaration's own `is_var` (removing a
/// stale `mutable_locals` entry left by an earlier `var`-declared
/// binding of the same name would otherwise silently outlive its own
/// declaration).
fn declare_local(
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  name: &str,
  ty: Type,
  is_var: bool,
) {
  env.insert(name.to_string(), ty);
  if is_var {
    mutable_locals.insert(name.to_string());
  } else {
    mutable_locals.remove(name);
  }
}

/// Plan 72's Decision log: the actual reassignment check shared by
/// every mutation form that targets an already-declared plain local —
/// `Stmt::Assign` (and, via parse-time desugaring, `+=`/`-=`/`*=`/`/=`/
/// `%=`), `Stmt::MultiAssign`, and `Stmt::OrAssign`/`Stmt::AndAssign`
/// (plan 43's `||=`/`&&=` — not named in the plan's own text, but the
/// identical mutation hazard: exempting them would leave a silent,
/// easy-to-miss hole in an otherwise-uniform rule). Called only after
/// the caller has already confirmed `name` exists in `env` (an
/// undefined-variable diagnostic always takes priority over an
/// immutability one).
fn check_mutable(
  name: &str,
  mutable_locals: &HashSet<String>,
  span: (usize, usize),
) -> Result<(), Diagnostic> {
  if mutable_locals.contains(name) {
    Ok(())
  } else {
    Err(Diagnostic::new(
      format!("cannot reassign immutable binding `{name}`, declared without `var`"),
      span,
    ))
  }
}

/// Type-checks one statement, threading a mutable local-variable
/// environment and the enclosing function's declared return type (used to
/// check every `return <expr>`, not just a trailing one). `in_loop` gates
/// `break`/`next` legality. `self_fields` is `Some` only inside a method
/// body (see `infer_expr_type`). `mutable_locals` (plan 72's Decision
/// log) is the companion set of names actually declared `var` — always
/// scoped, cloned, and discarded in lockstep with `env` itself, so a
/// `var`-marked name never leaks past the scope its own `Let` declared
/// it in.
#[allow(clippy::too_many_arguments)]
fn check_stmt(
  stmt: &Spanned<Stmt>,
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  // Plan 38: mirrors `in_loop` exactly — threaded through every
  // `check_block`/`check_stmt` call unchanged, forced `true` only by
  // `check_begin` for each `RescueClause.body` (never for the `begin`'s
  // own try `body` or its `ensure` body). Gates `retry`'s legality the
  // same way `in_loop` gates `break`/`next`.
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  match &stmt.node {
    // `Proc` is special-cased: the bare annotation carries no signature
    // (see `Type::Proc`'s doc comment), so instead of comparing against
    // `resolve_type("Proc", ...)`'s opaque placeholder, any actual
    // `Type::Proc(_, _)` is accepted and *that* — the real signature
    // inferred from the bound `Expr::Lambda` — is what's stored in `env`.
    Stmt::Let {
      name,
      ty,
      value,
      is_var,
    } if ty == "Proc" => {
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !matches!(actual, Type::Proc(_, _)) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name}: Proc = ...`: expected a Proc (lambda literal), found {actual:?}"
          ),
          value.span,
        ));
      }
      declare_local(env, mutable_locals, name, actual, *is_var);
      Ok(())
    }
    // `Supervisor` is special-cased the same way `Proc` is immediately
    // above (plan 57's Decision log) — the bare annotation carries no
    // per-child information (see `Type::Supervisor`'s own doc comment),
    // so instead of comparing against `resolve_type("Supervisor", ...)`
    // (which would fail — "Supervisor" is never a real `ClassInfo`),
    // any actual `Type::Supervisor(_)` is accepted and *that* — the
    // real tracked-children record inferred from the bound `Expr::
    // Supervise` — is what's stored in `env`.
    Stmt::Let {
      name,
      ty,
      value,
      is_var,
    } if ty == "Supervisor" => {
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !matches!(actual, Type::Supervisor(_)) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name}: Supervisor = ...`: expected a `supervise do ... end` block, found {actual:?}"
          ),
          value.span,
        ));
      }
      declare_local(env, mutable_locals, name, actual, *is_var);
      Ok(())
    }
    // `Array.new(size)`'s element type comes from the enclosing `Let`'s
    // own declared annotation (plan 25's Decision log) — special-cased
    // the same way `Proc` is above, since `infer_expr_type` alone has
    // no declared-type context available to it.
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::ArrayNew(size),
        ..
      },
      is_var,
    } => {
      let declared = resolve_type(ty, classes)?;
      if !matches!(declared, Type::Array(_)) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name}: {ty} = Array.new(...)`: `Array.new` produces an Array, not {declared:?}"
          ),
          stmt.span,
        ));
      }
      let size_ty = infer_expr_type(size, env, sigs, classes, self_fields, gctx)?;
      if size_ty != Type::Int64 {
        return Err(Diagnostic::new(
          format!("`Array.new` size must be Int64, found {size_ty:?}"),
          size.span,
        ));
      }
      declare_local(env, mutable_locals, name, declared, *is_var);
      Ok(())
    }
    // Plan 53's Decision log: `Ok`/`Err` construction, one of the three
    // expected-type-providing positions — a `Let`'s own declared type.
    Stmt::Let {
      name,
      ty,
      value: value @ Spanned {
        node: Expr::Ok(_) | Expr::Err(_),
        ..
      },
      is_var,
    } => {
      let declared = resolve_type(ty, classes)?;
      check_result_construction(&declared, value, env, sigs, classes, self_fields, gctx)?;
      declare_local(env, mutable_locals, name, declared, *is_var);
      Ok(())
    }
    // Plan 53's Decision log: `n: T = parse_int(s)?` — the `Let` half
    // of `?`'s two legal syntactic positions.
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::Try(inner),
        ..
      },
      is_var,
    } => {
      let unwrapped = check_try(inner, return_type, env, sigs, classes, self_fields, gctx)?;
      let declared = resolve_type(ty, classes)?;
      if !is_assignable(&unwrapped, &declared) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name}: {ty} = ...?`: declared type {declared:?}, unwrapped value has type {unwrapped:?}"
          ),
          inner.span,
        ));
      }
      declare_local(env, mutable_locals, name, declared, *is_var);
      Ok(())
    }
    // Plan 58's Decision log: `s: Stack[Int64] = Stack.new()` — resolves
    // via the MANGLED name directly rather than needing `generic_classes`
    // threaded into `check_stmt`'s own signature. `Expr::New`'s ordinary
    // arm below looks up `classes.get(class_name)` against the bare,
    // unmangled `"Stack"`, which is never registered there (only its
    // instantiations are) — a bare `Stack.new()` with no adjacent
    // expected-type-providing position (AC7) is rejected for free by
    // that same ordinary arm's existing "undefined class" diagnostic.
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::New(class_name, args),
        ..
      },
      is_var,
    } if matches!(ty, TypeExpr::Generic(base, _) if base == class_name) => {
      let declared = resolve_type(ty, classes)?;
      let Type::Class(mangled) = &declared else {
        unreachable!("resolve_type's generic-instantiation branch always returns Type::Class");
      };
      let info = classes
        .get(mangled)
        .expect("resolve_type already validated this mangled name exists in `classes`");
      match info.methods.get("initialize") {
        Some(sig) => check_args(
          "initialize",
          args,
          &sig.params,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        )?,
        None if args.is_empty() => {}
        None => {
          return Err(Diagnostic::new(
            format!(
              "`{class_name}.new` called with {} argument(s), but `{class_name}` declares no `initialize`",
              args.len()
            ),
            stmt.span,
          ));
        }
      }
      declare_local(env, mutable_locals, name, declared, *is_var);
      Ok(())
    }
    Stmt::Let {
      name,
      ty,
      value,
      is_var,
    } => {
      let declared = resolve_type(ty, classes)?;
      // Plan 73's Decision log: an `Option[T]`'s `Some`/`None`
      // construction, the fourth "expected-type-providing position"
      // this compiler now has (mirroring `Ok`/`Err`'s dedicated `Let`
      // arm above, generalized to any enum) — checked before the
      // ordinary `infer_expr_type` fallback below so a bare `None` (and
      // an ambiguous `Some(x)` across multiple `Option[T]`
      // instantiations) always resolves unambiguously here.
      if let Type::Enum(enum_name) = &declared {
        if check_enum_variant_construction(enum_name, value, env, sigs, classes, self_fields, gctx)?
        {
          declare_local(env, mutable_locals, name, declared, *is_var);
          return Ok(());
        }
      }
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name}: {ty} = ...`: declared type {declared:?}, value has type {actual:?}"
          ),
          value.span,
        ));
      }
      declare_local(env, mutable_locals, name, declared, *is_var);
      Ok(())
    }
    Stmt::SetField { name, value } => {
      let fields = self_fields.ok_or_else(|| {
        Diagnostic::new(
          format!("`@{name} = ...` used outside of a method body"),
          stmt.span,
        )
      })?;
      let declared = fields
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined field `@{name}`"), stmt.span))?;
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `@{name} = ...`: field declared {declared:?}, value has type {actual:?}"
          ),
          value.span,
        ));
      }
      Ok(())
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => check_set_index(array, index, value, env, sigs, classes, self_fields, gctx),
    // Plan 31: `name` must already be bound — this is a reassignment,
    // never a fresh declaration (the Decision log's whole reason this
    // is a distinct `Stmt` from `Let`). Checked against the *existing*
    // binding's type, same "no implicit conversion" rule `Let` and
    // every other assignment shape here already enforces.
    // Plan 53's Decision log: `Ok`/`Err` construction, one of the three
    // expected-type-providing positions — an `Assign`'s already-
    // recorded type.
    Stmt::Assign {
      name,
      value: value @ Spanned {
        node: Expr::Ok(_) | Expr::Err(_),
        ..
      },
    } => {
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), stmt.span))?;
      check_mutable(name, mutable_locals, stmt.span)?;
      check_result_construction(&declared, value, env, sigs, classes, self_fields, gctx)
    }
    // Plan 53's Decision log: `name = parse_int(s)?` — the `Assign`
    // half of `?`'s two legal syntactic positions.
    Stmt::Assign {
      name,
      value: Spanned {
        node: Expr::Try(inner),
        ..
      },
    } => {
      let unwrapped = check_try(inner, return_type, env, sigs, classes, self_fields, gctx)?;
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), stmt.span))?;
      check_mutable(name, mutable_locals, stmt.span)?;
      if !is_assignable(&unwrapped, &declared) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name} = ...?`: `{name}` has type {declared:?}, unwrapped value has type {unwrapped:?}"
          ),
          inner.span,
        ));
      }
      Ok(())
    }
    Stmt::Assign { name, value } => {
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"), stmt.span))?;
      check_mutable(name, mutable_locals, stmt.span)?;
      // Plan 73's Decision log: mirrors `Let`'s own identical addition
      // above — `Option[T]`'s `Some`/`None` construction against an
      // `Assign`'s already-recorded declared type.
      if let Type::Enum(enum_name) = &declared {
        if check_enum_variant_construction(enum_name, value, env, sigs, classes, self_fields, gctx)?
        {
          return Ok(());
        }
      }
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch in `{name} = ...`: `{name}` has type {declared:?}, value has type {actual:?}"
          ),
          value.span,
        ));
      }
      Ok(())
    }
    Stmt::MultiAssign { names, values } => check_multi_assign(
      names,
      values,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      gctx,
      stmt.span,
    ),
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields, gctx)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(
          format!(
            "`if` condition must be Boolean, found {cond_ty:?} (no truthy/falsy coercion — spec/GRAMMAR.md §5)"
          ),
          cond.span,
        ));
      }
      check_block(
        then_branch,
        env,
        mutable_locals,
        sigs,
        classes,
        self_fields,
        return_type,
        in_loop,
        in_rescue,
        yields_allowed,
        gctx,
      )?;
      if let Some(else_b) = else_branch {
        check_block(
          else_b,
          env,
          mutable_locals,
          sigs,
          classes,
          self_fields,
          return_type,
          in_loop,
          in_rescue,
          yields_allowed,
          gctx,
        )?;
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields, gctx)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(
          format!("`while` condition must be Boolean, found {cond_ty:?}"),
          cond.span,
        ));
      }
      check_block(
        body,
        env,
        mutable_locals,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    // Plan 53's Decision log: `Ok`/`Err` construction, the third of the
    // three expected-type-providing positions — a `Return`'s already-
    // threaded `return_type`.
    Stmt::Return(Some(
      e @ Spanned {
        node: Expr::Ok(_) | Expr::Err(_),
        ..
      },
    )) => check_result_construction(return_type, e, env, sigs, classes, self_fields, gctx),
    Stmt::Return(Some(e)) => {
      // Plan 73's Decision log: mirrors `Let`/`Assign`'s own identical
      // addition above — `Option[T]`'s `Some`/`None` construction
      // against a `Return`'s already-threaded declared return type, the
      // fourth "expected-type-providing position."
      if let Type::Enum(enum_name) = return_type {
        if check_enum_variant_construction(enum_name, e, env, sigs, classes, self_fields, gctx)? {
          return Ok(());
        }
      }
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&t, return_type) {
        return Err(Diagnostic::new(
          format!(
            "type mismatch: `return` value has type {t:?} but the enclosing function declares {return_type:?}"
          ),
          e.span,
        ));
      }
      Ok(())
    }
    Stmt::Return(None) => Ok(()),
    Stmt::Break => {
      if !in_loop {
        return Err(Diagnostic::new("`break` outside of a loop", stmt.span));
      }
      Ok(())
    }
    Stmt::Next => {
      if !in_loop {
        return Err(Diagnostic::new("`next` outside of a loop", stmt.span));
      }
      Ok(())
    }
    Stmt::Expr(e) => infer_expr_type(e, env, sigs, classes, self_fields, gctx).map(|_| ()),
    Stmt::Raise(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if !matches!(t, Type::Class(_)) {
        return Err(Diagnostic::new(
          format!("`raise` requires a class instance, found {t:?}"),
          e.span,
        ));
      }
      Ok(())
    }
    // Plan 34: legal only inside a function/method declaring
    // `block_param` — `yields_allowed` carries that down from
    // `check_function_body`/`check_method_body`. This generic pass has
    // no specific attached block to check `args`' types against yet
    // (no single fixed block signature exists — see the Decision log),
    // so it only verifies `args` are well-formed expressions (catching
    // an undefined variable, etc.); the real per-attachment arity/type
    // check happens separately, once per call site, in
    // `check_block_call_sites`.
    Stmt::Yield(args) => {
      if !yields_allowed {
        return Err(Diagnostic::new(
          "`yield` used outside of a function or method that declares a block parameter (`&name`)",
          stmt.span,
        ));
      }
      for a in args {
        infer_expr_type(a, env, sigs, classes, self_fields, gctx)?;
      }
      Ok(())
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => check_begin(
      body,
      rescues,
      ensure,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    ),
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => check_case(
      scrutinee,
      arms,
      else_body,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    ),
    // Plan 30: `elements`'s element type is unified exactly as
    // `infer_array_lit_type` already does for a bare array literal
    // (reject empty, reject heterogeneous) — `for` just also binds
    // `var` at that type for `body`, and threads `in_loop = true` so
    // `break`/`next` are legal inside it, same as `while`.
    Stmt::For {
      var,
      elements,
      body,
    } => {
      // `infer_array_lit_type` returns the literal's own `Array(elem)`
      // type, not the element type `var` should be bound at — unwrap
      // one layer.
      let Type::Array(elem_ty) =
        infer_array_lit_type(elements, env, sigs, classes, self_fields, gctx)?
      else {
        unreachable!("infer_array_lit_type always returns Type::Array")
      };
      // Plan 72's Decision log: `for`'s induction variable has no `var`
      // slot anywhere in this grammar — an ordinary `env.insert`, never
      // `declare_local`, so it's unconditionally immutable, the same
      // "no syntax to opt in, so it can't be mutable" reasoning a
      // parameter or a `case`/`rescue` pattern binding gets below.
      env.insert(var.clone(), *elem_ty);
      check_block(
        body,
        env,
        mutable_locals,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    // Plan 37: no first-class `Range` value — `start`/`end` are each
    // independently checked as `Int64` (rejecting any other type by
    // name, not silently coercing), `var` is bound at `Int64`
    // unconditionally for `body`, and `in_loop = true` is threaded
    // exactly as `Stmt::For`'s own arm does. A reverse range
    // (`start > end`) is deliberately not rejected here — that's a
    // runtime shape (a well-typed, zero-iteration loop), not a type
    // error.
    Stmt::ForRange {
      var,
      start,
      end,
      exclusive: _,
      body,
    } => {
      let start_ty = infer_expr_type(start, env, sigs, classes, self_fields, gctx)?;
      if start_ty != Type::Int64 {
        return Err(Diagnostic::new(
          format!("range start must be Int64, found {start_ty:?}"),
          start.span,
        ));
      }
      let end_ty = infer_expr_type(end, env, sigs, classes, self_fields, gctx)?;
      if end_ty != Type::Int64 {
        return Err(Diagnostic::new(
          format!("range end must be Int64, found {end_ty:?}"),
          end.span,
        ));
      }
      // Plan 72's Decision log: same reasoning as `Stmt::For`'s own
      // arm above — no `var` slot on a `for...in start..end` induction
      // variable either, so it's unconditionally immutable.
      env.insert(var.clone(), Type::Int64);
      check_block(
        body,
        env,
        mutable_locals,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    // Plan 38: legal only inside a `rescue` clause's own body — see
    // `check_begin`, which forces `in_rescue = true` only there, never
    // for the `begin`'s own try `body` or its `ensure` body.
    Stmt::Retry => {
      if !in_rescue {
        return Err(Diagnostic::new(
          "`retry` outside of a rescue body",
          stmt.span,
        ));
      }
      Ok(())
    }
    // Plan 53's Decision log: `case scrutinee when Ok(v) ... when
    // Err(e) ... end` — a small, dedicated form, independent of plan
    // 52's `Stmt::Case`/`CasePattern` mechanism.
    Stmt::MatchResult {
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
    } => check_match_result(
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    ),
  }
}

/// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan 20's
/// Decision log): the scrutinee and every `when` value must be
/// `Int64` — not `Float64`/`String`/`Class` — value-matched via the
/// same `CompareOp::Eq` `Expr::Compare` already performs, not
/// `spec/GRAMMAR.md`'s eventual method-dispatched `===`.
#[allow(clippy::too_many_arguments)]
fn check_case(
  scrutinee: &Spanned<Expr>,
  arms: &[CaseArm],
  else_body: &Option<Vec<Spanned<Stmt>>>,
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let scrutinee_ty = infer_expr_type(scrutinee, env, sigs, classes, self_fields, gctx)?;
  // Plan 52's Decision log: the first bounded, fully-known-at-compile-
  // time domain a `case` scrutinee has ever had — real exhaustiveness
  // checking over the enum's own closed variant set.
  if let Type::Enum(enum_name) = &scrutinee_ty {
    let variants = classes
      .get(enum_name)
      .and_then(|c| c.enum_variants.as_ref())
      .expect("Type::Enum is only ever constructed for a registered enum");
    let mut matched: HashSet<String> = HashSet::new();
    for (pattern, body) in arms {
      let CasePattern::Variant { name, bindings } = pattern else {
        return Err(Diagnostic::new(
          format!("case over an enum `{enum_name}` scrutinee cannot use a value pattern"),
          scrutinee.span,
        ));
      };
      let Some((_, field_types)) = variants.iter().find(|(vn, _)| vn == name) else {
        return Err(Diagnostic::new(
          format!("`{name}` is not a variant of enum `{enum_name}`"),
          scrutinee.span,
        ));
      };
      if !matched.insert(name.clone()) {
        return Err(Diagnostic::new(
          format!("`case` over `{enum_name}` matches variant `{name}` more than once"),
          scrutinee.span,
        ));
      }
      if bindings.len() != field_types.len() {
        return Err(Diagnostic::new(
          format!(
            "pattern `{name}` expects {} binding(s), found {}",
            field_types.len(),
            bindings.len()
          ),
          scrutinee.span,
        ));
      }
      // Plan 52's Decision log: a real, disclosed departure from this
      // compiler's standing flat-scoping convention — each arm's
      // bindings type-check against a *cloned* extension of `env`,
      // discarded once this arm's body is checked, so a binding never
      // leaks into a later arm, the `else` body, or a statement after
      // the `case` ends.
      // Plan 72's Decision log: `bname` is an ordinary `env.insert` (not
      // `declare_local`) into the cloned `arm_env` — a `case`/`match`
      // pattern binding has no `var` slot in this grammar, so it's
      // unconditionally immutable, and `arm_mutable` is cloned from the
      // OUTER `mutable_locals` (not left empty) so a `var`-declared name
      // visible before the `case` stays reassignable inside each arm.
      let mut arm_env = env.clone();
      let mut arm_mutable = mutable_locals.clone();
      for (bname, bty) in bindings.iter().zip(field_types.iter()) {
        arm_env.insert(bname.clone(), bty.clone());
      }
      check_block(
        body,
        &mut arm_env,
        &mut arm_mutable,
        sigs,
        classes,
        self_fields,
        return_type,
        in_loop,
        in_rescue,
        yields_allowed,
        gctx,
      )?;
    }
    if else_body.is_none() {
      let missing: Vec<&str> = variants
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| !matched.contains(*n))
        .collect();
      if !missing.is_empty() {
        return Err(Diagnostic::new(
          format!(
            "`case` over `{enum_name}` does not cover variant `{}`",
            missing.join("`, `")
          ),
          scrutinee.span,
        ));
      }
    }
    if let Some(else_b) = else_body {
      check_block(
        else_b,
        env,
        mutable_locals,
        sigs,
        classes,
        self_fields,
        return_type,
        in_loop,
        in_rescue,
        yields_allowed,
        gctx,
      )?;
    }
    return Ok(());
  }
  if scrutinee_ty != Type::Int64 {
    return Err(Diagnostic::new(
      format!("`case` scrutinee must be Int64 or an enum type, found {scrutinee_ty:?}"),
      scrutinee.span,
    ));
  }
  for (pattern, body) in arms {
    let CasePattern::Values(values) = pattern else {
      return Err(Diagnostic::new(
        "case over an Int64 scrutinee cannot use a variant pattern",
        scrutinee.span,
      ));
    };
    for v in values {
      let value_ty = infer_expr_type(v, env, sigs, classes, self_fields, gctx)?;
      if value_ty != Type::Int64 {
        return Err(Diagnostic::new(
          format!("`when` value must be Int64, found {value_ty:?}"),
          v.span,
        ));
      }
    }
    check_block(
      body,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  if let Some(else_b) = else_body {
    check_block(
      else_b,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
}

/// Plan 53's Decision log: `case scrutinee when Ok(v) ... when Err(e)
/// ... end` — a fixed, two-armed `Result[T, E]` destructuring form.
/// Unlike plan 38's bare-`rescue` case (no universal root class to
/// type an unnamed binding at), `Result`'s `T`/`E` are always
/// statically known here, so both `ok_var`/`err_var` are real, usable
/// bindings — each type-checked against a *cloned* extension of `env`,
/// discarded once its own arm's body is checked, mirroring plan 52's
/// own per-arm-scoped pattern bindings exactly.
#[allow(clippy::too_many_arguments)]
fn check_match_result(
  scrutinee: &Spanned<Expr>,
  ok_var: &str,
  ok_body: &[Spanned<Stmt>],
  err_var: &str,
  err_body: &[Spanned<Stmt>],
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let scrutinee_ty = infer_expr_type(scrutinee, env, sigs, classes, self_fields, gctx)?;
  let Type::Result(t_ty, e_ty) = &scrutinee_ty else {
    return Err(Diagnostic::new(
      format!(
        "`case ... when Ok(...) when Err(...)` requires a `Result[T, E]` scrutinee, found {scrutinee_ty:?}"
      ),
      scrutinee.span,
    ));
  };
  // Plan 72's Decision log: `ok_var`/`err_var` are ordinary `env.insert`s
  // (not `declare_local`) into their own cloned env — no `var` slot
  // exists on either binding in this grammar, so both are
  // unconditionally immutable; each `mutable_locals` clone still
  // carries forward whatever was already reassignable before the
  // `match`, the same "clone, don't reset" precedent `check_case`'s own
  // `arm_mutable` sets.
  let mut ok_env = env.clone();
  let mut ok_mutable = mutable_locals.clone();
  ok_env.insert(ok_var.to_string(), (**t_ty).clone());
  check_block(
    ok_body,
    &mut ok_env,
    &mut ok_mutable,
    sigs,
    classes,
    self_fields,
    return_type,
    in_loop,
    in_rescue,
    yields_allowed,
    gctx,
  )?;
  let mut err_env = env.clone();
  let mut err_mutable = mutable_locals.clone();
  err_env.insert(err_var.to_string(), (**e_ty).clone());
  check_block(
    err_body,
    &mut err_env,
    &mut err_mutable,
    sigs,
    classes,
    self_fields,
    return_type,
    in_loop,
    in_rescue,
    yields_allowed,
    gctx,
  )?;
  Ok(())
}

/// `begin body rescue Type => e ... [rescue => e2 ...] [ensure ...] end`
/// (plan 38's Decision log). Flat scoping, same as everything else in
/// this compiler (plan 07's Decision log) — each typed clause's `var`
/// joins the same environment an `if`/`while` body's `Let`s already
/// flow into, not a fresh scope. A bare clause's `var` is never
/// inserted into `env` at all (no universal root class exists to type
/// it at — any reference inside that clause's body falls through to
/// the ordinary "undefined variable" diagnostic). `in_rescue` is
/// forced `true` only for each `RescueClause.body` — left unchanged
/// for the try `body` and the `ensure` body, so `retry` is illegal in
/// both, matching Ruby's own restriction.
#[allow(clippy::too_many_arguments)]
fn check_begin(
  body: &[Spanned<Stmt>],
  rescues: &[RescueClause],
  ensure: &Option<Vec<Spanned<Stmt>>>,
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  check_block(
    body,
    env,
    mutable_locals,
    sigs,
    classes,
    self_fields,
    return_type,
    in_loop,
    in_rescue,
    yields_allowed,
    gctx,
  )?;
  for rescue in rescues {
    if let Some(class_name) = &rescue.class_name {
      let rescue_ty = resolve_type(&TypeExpr::Named(class_name.clone()), classes)?;
      if !matches!(rescue_ty, Type::Class(_)) {
        // `RescueClause` carries no span of its own (ast.rs never wraps
        // it in `Spanned` — see plan 22's own scope note); an honest
        // `(0, 0)`.
        return Err(Diagnostic::new(
          format!("`rescue {class_name}` must name a class, found {rescue_ty:?}"),
          (0, 0),
        ));
      }
      // Plan 72's Decision log: flat scoping (no clone — matches this
      // whole clause's existing "joins the same environment" doc comment
      // above), and an ordinary `env.insert`, never `declare_local` — a
      // `rescue` exception variable has no `var` slot in this grammar,
      // so it's unconditionally immutable.
      env.insert(rescue.var.clone(), rescue_ty);
    }
    check_block(
      &rescue.body,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      true,
      yields_allowed,
      gctx,
    )?;
  }
  if let Some(ensure_body) = ensure {
    check_block(
      ensure_body,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_block(
  stmts: &[Spanned<Stmt>],
  env: &mut HashMap<String, Type>,
  mutable_locals: &mut HashSet<String>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    check_stmt(
      stmt,
      env,
      mutable_locals,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
}

/// Plan 65's Decision log: a cross-actor send is legal as a bare
/// statement in ANY position, including the trailing one
/// (`check_message_safety_expr_stmt`'s own doc comment already
/// establishes this for its own, differently-scoped purpose) — its
/// real type is `Result[Void, SendError]`, not `Void`, but a fire-
/// and-forget send discards that value uniformly regardless of
/// position, the same way it always discarded `Void`. A trailing one
/// is therefore NOT an implicit return of that `Result` and must be
/// exempted from `check_implicit_return`'s own declared-return-type
/// match below, or every pre-existing `-> Void` method/function whose
/// body happens to END with a bare send (previously trivially
/// matching `Void == Void`) would newly fail to type-check — a real,
/// disclosed regression this leaf's own new test caught, corrected
/// here rather than left as an unstated breaking change the Decision
/// log's own "additive at the source level" claim would otherwise be
/// wrong about.
fn is_cross_actor_send(
  e: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  self_fields: Option<&HashMap<String, Type>>,
  classes: &HashMap<String, ClassInfo>,
) -> bool {
  let Expr::MethodCall(recv, method, _) = &e.node else {
    return false;
  };
  if method == "register" {
    return false;
  }
  let recv_ty = match &recv.node {
    Expr::Ident(name) if name == "self" => return false,
    Expr::Ident(name) => env.get(name),
    Expr::InstanceVar(field) => self_fields.and_then(|f| f.get(field)),
    _ => None,
  };
  matches!(recv_ty, Some(Type::Class(cn)) if classes.get(cn).is_some_and(|c| c.is_actor))
}

/// Checks the final-statement implicit-return rule shared by free
/// functions and methods (Ruby-style: a body whose last statement is a
/// bare expression returns that expression's value).
#[allow(clippy::too_many_arguments)]
fn check_implicit_return(
  body: &[Spanned<Stmt>],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  declared_return: &Type,
  owner_name: &str,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  if let Some(Spanned {
    node: Stmt::Expr(e),
    ..
  }) = body.last()
  {
    if is_cross_actor_send(e, env, self_fields, classes) {
      return Ok(());
    }
    // Plan 73's Decision log: a bare trailing `Some(x)`/`None` (no
    // `return` keyword) as a function body's own final, implicit-return
    // expression — this plan's own concrete target proof needs exactly
    // this shape (`fn find(...): Option[Int64] do ... None end`). A
    // real, disclosed EXTENSION beyond `Result[T, E]`'s own narrower
    // three-position scope (`Ok`/`Err`'s doc comment explicitly lists
    // only `Let`/`Assign`/`Return`, not a bare implicit return) — this
    // fourth position is added here specifically because `Option[T]`'s
    // own headline proof requires it, not retrofitted onto `Result`.
    if let Type::Enum(enum_name) = declared_return {
      if check_enum_variant_construction(enum_name, e, env, sigs, classes, self_fields, gctx)? {
        return Ok(());
      }
    }
    let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
    if t != *declared_return {
      return Err(Diagnostic::new(
        format!(
          "type mismatch in `{owner_name}`: body has type {t:?} but declared return type is {declared_return:?}"
        ),
        e.span,
      ));
    }
  }
  Ok(())
}

/// Plan 56 (compile-time message safety) — Pony-lite, not Pony (see
/// this plan's own Decision log): a cross-actor message argument is
/// legal iff it's a value type, a freshly constructed reference, or a
/// named local provably not read again by the sender after the send.
///
/// A real, disclosed architectural adaptation of the plan's own
/// suggested integration point: rather than threading a new `moved`
/// parameter through `check_stmt`/`check_block`/`infer_expr_type`'s
/// entire pervasive call graph (`infer_expr_type` alone is this file's
/// single most-called function), this runs as its own separate,
/// read-only pass, called once from `check_function_body`/`check_
/// method_body` right after their own ordinary `check_block` call
/// already succeeded — reusing that same, by-then fully-populated
/// `env` unchanged (this compiler's own flat, function-wide scoping —
/// no shadowing exists anywhere, confirmed by `Stmt::Let`'s own
/// already-declared-name rejection — means every `Let`-bound name's
/// type is already known and stable for the rest of the function on
/// every path, by the time the ordinary pass finishes, regardless of
/// which branch textually declared it). Mirrors `check_stmt`'s own
/// control-flow structure (`If`/`Case`/`Begin`/`While`/`For`/
/// `ForRange`/`MatchResult`) for the moved-set fork/union/loop-
/// conservatism rules the Decision log specifies, but never mutates
/// `env` itself and touches no other function's signature in this
/// file. Scoped to function/method bodies only, matching the plan's
/// own literal target state — a bare top-level `Item::Stmt` send is a
/// real, disclosed gap this plan doesn't close (every one of this
/// plan's own worked examples wraps its send inside `def main()`).
fn check_message_safety(
  body: &[Spanned<Stmt>],
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  moved: &mut HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  for stmt in body {
    check_message_safety_stmt(stmt, env, classes, moved)?;
  }
  Ok(())
}

/// A loop body is checked once, then checked AGAIN against the
/// moved-set the first pass produced (Decision log: a second iteration
/// could reach a first-iteration's send before a later-in-text use
/// from that same iteration actually runs) — "treat the body as
/// concatenated with itself once," implemented directly. The whole
/// loop then poisons every local it sent for all code after the loop
/// exits, the same conservative-merge posture branches get.
fn check_loop_body(
  body: &[Spanned<Stmt>],
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  moved: &mut HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  let mut first_pass = moved.clone();
  check_message_safety(body, env, classes, &mut first_pass)?;
  let mut second_pass = first_pass.clone();
  check_message_safety(body, env, classes, &mut second_pass)?;
  moved.extend(second_pass);
  Ok(())
}

fn check_message_safety_stmt(
  stmt: &Spanned<Stmt>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  moved: &mut HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  match &stmt.node {
    Stmt::Let { value, .. } | Stmt::SetField { value, .. } | Stmt::Assign { value, .. } => {
      expr_moved_read(value, moved)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      expr_moved_read(array, moved)?;
      expr_moved_read(index, moved)?;
      expr_moved_read(value, moved)
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        expr_moved_read(v, moved)?;
      }
      Ok(())
    }
    Stmt::Return(Some(e)) | Stmt::Raise(e) => expr_moved_read(e, moved),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => Ok(()),
    Stmt::Yield(args) => {
      for a in args {
        expr_moved_read(a, moved)?;
      }
      Ok(())
    }
    Stmt::Expr(e) => check_message_safety_expr_stmt(e, env, classes, moved),
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      expr_moved_read(cond, moved)?;
      let mut then_moved = moved.clone();
      check_message_safety(then_branch, env, classes, &mut then_moved)?;
      let mut else_moved = moved.clone();
      if let Some(else_b) = else_branch {
        check_message_safety(else_b, env, classes, &mut else_moved)?;
      }
      moved.extend(then_moved);
      moved.extend(else_moved);
      Ok(())
    }
    Stmt::While { cond, body } => {
      expr_moved_read(cond, moved)?;
      check_loop_body(body, env, classes, moved)
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        expr_moved_read(e, moved)?;
      }
      check_loop_body(body, env, classes, moved)
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      expr_moved_read(start, moved)?;
      expr_moved_read(end, moved)?;
      check_loop_body(body, env, classes, moved)
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      expr_moved_read(scrutinee, moved)?;
      let mut union = HashMap::new();
      for (_, arm_body) in arms {
        let mut arm_moved = moved.clone();
        check_message_safety(arm_body, env, classes, &mut arm_moved)?;
        union.extend(arm_moved);
      }
      if let Some(else_b) = else_body {
        let mut else_moved = moved.clone();
        check_message_safety(else_b, env, classes, &mut else_moved)?;
        union.extend(else_moved);
      }
      moved.extend(union);
      Ok(())
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      let mut body_moved = moved.clone();
      check_message_safety(body, env, classes, &mut body_moved)?;
      let mut union = body_moved;
      for r in rescues {
        let mut r_moved = moved.clone();
        check_message_safety(&r.body, env, classes, &mut r_moved)?;
        union.extend(r_moved);
      }
      moved.extend(union);
      // `ensure` always runs, on every exit path — checked against
      // `moved` as it now stands post-union, the same conservative
      // posture branches get.
      if let Some(ensure_b) = ensure {
        check_message_safety(ensure_b, env, classes, moved)?;
      }
      Ok(())
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      expr_moved_read(scrutinee, moved)?;
      let mut ok_moved = moved.clone();
      check_message_safety(ok_body, env, classes, &mut ok_moved)?;
      let mut err_moved = moved.clone();
      check_message_safety(err_body, env, classes, &mut err_moved)?;
      moved.extend(ok_moved);
      moved.extend(err_moved);
      Ok(())
    }
  }
}

/// `Stmt::Expr(e)` is where a cross-actor send (`receiver.method(args)`
/// with an actor-typed receiver — plan 55's assumed contract, `.spawn`
/// forces `Void` on every non-`initialize` actor method, so a send can
/// only ever appear as a bare statement) is recognized; every other
/// bare-expression statement just gets the ordinary moved-read check.
fn check_message_safety_expr_stmt(
  e: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  moved: &mut HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  if let Expr::MethodCall(recv, method, args) = &e.node {
    if let Expr::Ident(recv_name) = &recv.node {
      if let Some(Type::Class(class_name)) = env.get(recv_name) {
        if classes.get(class_name).is_some_and(|c| c.is_actor) {
          for (i, arg) in args.iter().enumerate() {
            // Plan 56's own existing liveness check runs first,
            // completely unchanged — plan 60's own wire-safety
            // predicate (Design decision 2) is a second, independent
            // check layered on AFTER it, never in place of it.
            check_message_arg(arg, i, method, env, moved)?;
            check_wire_safety(arg, i, method, env, classes)?;
          }
          return Ok(());
        }
      }
    }
  }
  expr_moved_read(e, moved)
}

/// `leaf-payload-classification`'s four buckets, in the order the
/// Decision log states them. Bucket 3 (a named-local reference) is the
/// only one that both consults AND updates `moved`.
fn check_message_arg(
  arg: &Spanned<Expr>,
  index: usize,
  method: &str,
  env: &HashMap<String, Type>,
  moved: &mut HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  match &arg.node {
    // Bucket 2: trivially fresh — always legal, no prior binding for
    // anything else to read afterward. Each constructor's own inner
    // arguments are still real reads in their own right (e.g.
    // `LogMessage.new(some_moved_local)`), checked recursively.
    Expr::New(_, inner) | Expr::Spawn(_, inner) => {
      for a in inner {
        expr_moved_read(a, moved)?;
      }
      Ok(())
    }
    Expr::ArrayLit(elems) => {
      for e in elems {
        expr_moved_read(e, moved)?;
      }
      Ok(())
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        expr_moved_read(k, moved)?;
        expr_moved_read(v, moved)?;
      }
      Ok(())
    }
    Expr::ArrayNew(size) => expr_moved_read(size, moved),
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          expr_moved_read(e, moved)?;
        }
      }
      Ok(())
    }
    Expr::Lambda { .. } | Expr::StringLit(_) => Ok(()),
    // Bucket 1: value types, always legal by literal shape.
    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::SymbolLit(_) => Ok(()),
    // Bucket 3: a named local — value-typed locals are exempt (still
    // bucket 1, just via a variable instead of a literal); every other
    // type is subject to the liveness check, then recorded as a new
    // move.
    Expr::Ident(name) => {
      if let Some(&send_span) = moved.get(name) {
        return Err(Diagnostic::new(
          format!(
            "message-safety: local `{name}` cannot be used again — it was already sent to an \
             actor (byte offset {}) and message payloads are a one-way transfer",
            send_span.0
          ),
          arg.span,
        ));
      }
      let is_value_type = matches!(
        env.get(name),
        Some(Type::Int64) | Some(Type::Float64) | Some(Type::Boolean) | Some(Type::Symbol)
      );
      if !is_value_type {
        moved.insert(name.clone(), arg.span);
      }
      Ok(())
    }
    // Bucket 4: aliasing-shape — rejected outright, not analyzed (see
    // this plan's own Decision log for why `@field`/`arr[i]`/a nested
    // call result can't be proven unaliased by this compiler).
    Expr::InstanceVar(_) | Expr::Index(_, _) | Expr::Call(_, _) | Expr::MethodCall(_, _, _) => {
      Err(Diagnostic::new(
        format!(
          "message-safety: argument {} to `{method}` must be a value, a freshly constructed \
           value, or a local variable used for the last time — `@field`/`array[i]`/a nested call \
           result cannot be proven unaliased by this compiler",
          index + 1
        ),
        arg.span,
      ))
    }
    // Every other shape (arithmetic/comparison/bitwise, `Ok`/`Err`/
    // `Try`, `TupleLit`, `SafeCall`) reduces to a value in every case
    // this type system allows here — the same conservative-safe
    // recursive-read default every OTHER (non-message-argument)
    // expression position in this whole pass already gets.
    _ => expr_moved_read(arg, moved),
  }
}

/// Plan 60's Decision log — Design decision 2: a second, independent,
/// purely-type-driven predicate, layered on top of (never instead of)
/// `check_message_arg`'s own liveness check above — applied to every
/// cross-actor send argument whose receiver's STATIC type is an actor
/// (sema can't know local-vs-remote statically, so this is
/// conservative: it runs for every actor-typed receiver, not only ones
/// proven remote). Determines the argument's type from its own AST
/// shape + `env` directly (the same finite set of shapes `check_
/// message_arg`'s own bucket match already discriminates) rather than
/// the general `infer_expr_type` — sound here specifically because
/// `check_message_arg`'s bucket-4 rejection above already rules out
/// every shape (`@field`/`arr[i]`/a nested call result) whose type
/// isn't recoverable this cheaply.
fn check_wire_safety(
  arg: &Spanned<Expr>,
  index: usize,
  method: &str,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<(), Diagnostic> {
  let ty = match &arg.node {
    Expr::Int(_) => Some(Type::Int64),
    Expr::Float(_) => Some(Type::Float64),
    Expr::Bool(_) => Some(Type::Boolean),
    Expr::SymbolLit(_) => Some(Type::Symbol),
    Expr::StringLit(_) | Expr::Interpolate(_) => Some(Type::String),
    Expr::New(class_name, _) => Some(Type::Class(class_name.clone())),
    // An actor reference — `is_wire_safe_type`'s own `is_actor` check
    // below rejects this uniformly with `Expr::Ident` naming an
    // already-`.spawn`ed/`.remote`d actor local.
    Expr::Spawn(class_name, _) => Some(Type::Class(class_name.clone())),
    Expr::ArrayLit(_) | Expr::ArrayNew(_) => Some(Type::Array(Box::new(Type::Void))),
    Expr::HashLit(_) => Some(Type::Hash(Box::new(Type::Void), Box::new(Type::Void))),
    Expr::Lambda { .. } => Some(Type::Proc(Vec::new(), Box::new(Type::Void))),
    Expr::Ident(name) => env.get(name).cloned(),
    _ => None,
  };
  let Some(ty) = ty else {
    return Ok(());
  };
  let mut seen = HashSet::new();
  if !is_wire_safe_type(&ty, classes, &mut seen) {
    return Err(Diagnostic::new(
      format!(
        "message-safety: argument {} to `{method}` has type {ty:?}, which is not wire-safe — only \
         Int64/Float64/Boolean/Symbol/String, or a class whose fields are all recursively \
         wire-safe, can cross a (possibly remote) actor boundary; Array[_]/Hash[_,_]/Proc/an actor \
         reference are not",
        index + 1
      ),
      arg.span,
    ));
  }
  Ok(())
}

/// `Int64`/`Float64`/`Boolean`/`Symbol` (plan 56's own value types) and
/// `String` are wire-safe directly; a `Class(name)` is wire-safe iff
/// every one of its OWN flattened fields (`ClassLayout`'s existing
/// field list — plan 08/32's metadata, reused verbatim, not
/// recomputed) is, walked recursively. `seen` guards a self-referential
/// class (`Node { next: Node }`) from infinite recursion — a real,
/// disclosed limitation: a cyclic class is simply never wire-safe in
/// v1, rather than this plan attempting real cycle-aware encoding.
fn is_wire_safe_type(
  ty: &Type,
  classes: &HashMap<String, ClassInfo>,
  seen: &mut HashSet<String>,
) -> bool {
  match ty {
    Type::Int64 | Type::Float64 | Type::Boolean | Type::Symbol | Type::String => true,
    Type::Class(name) => {
      let Some(info) = classes.get(name) else {
        return false;
      };
      if info.is_actor {
        return false;
      }
      if !seen.insert(name.clone()) {
        return false;
      }
      info
        .fields
        .values()
        .all(|ft| is_wire_safe_type(ft, classes, seen))
    }
    _ => false,
  }
}

/// Any AST position that reads `Expr::Ident(name)` where `name` is a
/// key in `moved` is the rejection (Decision log: "any use" means
/// literally any position, not just a bare-statement one).
fn expr_moved_read(
  expr: &Spanned<Expr>,
  moved: &HashMap<String, (usize, usize)>,
) -> Result<(), Diagnostic> {
  match &expr.node {
    Expr::Ident(name) => {
      if let Some(&send_span) = moved.get(name) {
        return Err(Diagnostic::new(
          format!(
            "message-safety: local `{name}` cannot be used again — it was already sent to an \
             actor (byte offset {}) and message payloads are a one-way transfer",
            send_span.0
          ),
          expr.span,
        ));
      }
      Ok(())
    }
    Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::InstanceVar(_) => Ok(()),
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
      expr_moved_read(a, moved)?;
      expr_moved_read(b, moved)
    }
    Expr::Compare(a, _, b) => {
      expr_moved_read(a, moved)?;
      expr_moved_read(b, moved)
    }
    Expr::Neg(a)
    | Expr::Not(a)
    | Expr::BitNot(a)
    | Expr::ArrayNew(a)
    | Expr::Ok(a)
    | Expr::Err(a)
    | Expr::Try(a)
    | Expr::Comptime(a) => expr_moved_read(a, moved),
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        expr_moved_read(a, moved)?;
      }
      Ok(())
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        expr_moved_read(v, moved)?;
      }
      Ok(())
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      expr_moved_read(recv, moved)?;
      for a in args {
        expr_moved_read(a, moved)?;
      }
      Ok(())
    }
    Expr::Coalesce(a, b) => {
      expr_moved_read(a, moved)?;
      expr_moved_read(b, moved)
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        expr_moved_read(e, moved)?;
      }
      Ok(())
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        expr_moved_read(k, moved)?;
        expr_moved_read(v, moved)?;
      }
      Ok(())
    }
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          expr_moved_read(e, moved)?;
        }
      }
      Ok(())
    }
    // A lambda's own body is a real, disclosed gap this plan doesn't
    // close — walking a nested Stmt list from an Expr-position
    // function needs its own dedicated traversal this plan's own
    // worked examples never exercise (neither uses a lambda at all).
    Expr::Lambda { .. } => Ok(()),
    // Plan 57: same disclosed gap as `Expr::Lambda` immediately above,
    // for the identical reason — a `supervise do ... end` body is a
    // nested `Stmt` list, and this plan's own worked examples never
    // send a moved-out local as a supervised `.spawn` argument.
    Expr::Supervise(_) => Ok(()),
    Expr::Remote { addr, name, .. } => {
      expr_moved_read(addr, moved)?;
      expr_moved_read(name, moved)
    }
    Expr::Locate { key, args, .. } => {
      expr_moved_read(key, moved)?;
      for a in args {
        expr_moved_read(a, moved)?;
      }
      Ok(())
    }
  }
}

/// Plan 63's `leaf-sema-purity-check`: identifies one `pure`-claimed
/// top-level function (`Function`) or one `pure`-claimed method
/// (`Method(class_or_module_name, method_name)`) — the call graph's
/// node type. Two functions/methods with the same name in different
/// classes are genuinely distinct nodes; a bare top-level function and
/// a same-named method are also genuinely distinct (`sigs` vs.
/// `classes[_].methods` are already two separate namespaces
/// everywhere else in this file).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PureNode {
  Function(String),
  Method(String, String),
}

fn pure_node_display_name(node: &PureNode) -> String {
  match node {
    PureNode::Function(name) => name.clone(),
    PureNode::Method(owner, name) => format!("{owner}#{name}"),
  }
}

/// One `pure`-claimed function/method, gathered once up front so the
/// SCC/verification passes below never need to re-walk `program.items`.
struct PureCandidate<'a> {
  node: PureNode,
  f: &'a Function,
  self_fields: Option<&'a HashMap<String, Type>>,
}

/// Rebuilds the exact `env`/`declared_return` `check_function_body`/
/// `check_method_body` already built and successfully checked this
/// body against, earlier in `check_program` — re-running `check_block`
/// here (ignoring its `Result`, since that earlier, identical call is
/// this pass's own precondition for even being reached) is cheaper
/// than threading a second, `pure`-specific environment-tracking
/// scheme through this file, and keeps `check_purity_expr`'s own
/// actor-receiver detection on the exact same "`Ident` receiver found
/// in `env`" mechanism `check_message_safety_expr_stmt` already
/// established (see its own doc comment) rather than a second,
/// independently-maintained copy.
fn rebuild_purity_env(
  f: &Function,
  self_fields: Option<&HashMap<String, Type>>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> (HashMap<String, Type>, Type) {
  let mut env = HashMap::new();
  for p in &f.params {
    if let Ok(t) = resolve_type(&p.ty, classes) {
      env.insert(p.name.clone(), t);
    }
  }
  if self_fields.is_none() {
    if let Some(p) = &f.splat_param {
      if let Ok(elem_ty) = resolve_type(&p.ty, classes) {
        env.insert(p.name.clone(), Type::Array(Box::new(elem_ty)));
      }
    }
  }
  let declared_return = if self_fields.is_some() {
    resolve_type(&f.return_type, classes).unwrap_or(Type::Void)
  } else {
    resolve_return_type(&f.return_type, classes).unwrap_or(Type::Void)
  };
  // Plan 72: this re-run's result is discarded (see the doc comment
  // above) and `mutable_locals` is never inspected afterward — a fresh,
  // empty set is enough.
  let mut mutable_locals: HashSet<String> = HashSet::new();
  let _ = check_block(
    &f.body,
    &mut env,
    &mut mutable_locals,
    sigs,
    classes,
    self_fields,
    &declared_return,
    false,
    false,
    f.block_param.is_some(),
    gctx,
  );
  (env, declared_return)
}

/// A plain DFS-based Tarjan SCC pass (Decision log: no existing SCC
/// utility anywhere in this file, and the call graph is small enough
/// that a new Cargo dependency isn't justified). `strongconnect` is a
/// nested `fn`, not a closure, specifically so `adjacency` (never
/// mutated) and the five pieces of mutable DFS state can be borrowed
/// independently across the recursive call — a closure capturing
/// `&mut self`-style state hits the classic "recursing while holding a
/// borrow of the same struct" conflict; separate parameters don't.
/// Returns each SCC's member indices, in an order where a component
/// only ever calls into components that already appear EARLIER in the
/// returned `Vec` — Tarjan's own finishing order already gives this
/// for free (a callee's component is always fully explored, and thus
/// already pushed onto `result`, before its caller's own component
/// closes), which is exactly the "callees' components decided before
/// their callers'" processing order `check_purity` needs.
fn tarjan_scc(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
  let n = adjacency.len();
  let mut index_counter = 0usize;
  let mut indices: Vec<Option<usize>> = vec![None; n];
  let mut lowlinks: Vec<usize> = vec![0; n];
  let mut on_stack: Vec<bool> = vec![false; n];
  let mut stack: Vec<usize> = Vec::new();
  let mut result: Vec<Vec<usize>> = Vec::new();

  #[allow(clippy::too_many_arguments)]
  fn strongconnect(
    v: usize,
    adjacency: &[Vec<usize>],
    index_counter: &mut usize,
    indices: &mut [Option<usize>],
    lowlinks: &mut [usize],
    on_stack: &mut [bool],
    stack: &mut Vec<usize>,
    result: &mut Vec<Vec<usize>>,
  ) {
    indices[v] = Some(*index_counter);
    lowlinks[v] = *index_counter;
    *index_counter += 1;
    stack.push(v);
    on_stack[v] = true;
    for &w in &adjacency[v] {
      if indices[w].is_none() {
        strongconnect(
          w,
          adjacency,
          index_counter,
          indices,
          lowlinks,
          on_stack,
          stack,
          result,
        );
        lowlinks[v] = lowlinks[v].min(lowlinks[w]);
      } else if on_stack[w] {
        lowlinks[v] = lowlinks[v].min(indices[w].expect("w has an index — just checked Some"));
      }
    }
    if lowlinks[v] == indices[v].expect("v was just assigned Some above") {
      let mut scc = Vec::new();
      loop {
        let w = stack.pop().expect("v itself is still on the stack");
        on_stack[w] = false;
        scc.push(w);
        if w == v {
          break;
        }
      }
      result.push(scc);
    }
  }

  for v in 0..n {
    if indices[v].is_none() {
      strongconnect(
        v,
        adjacency,
        &mut index_counter,
        &mut indices,
        &mut lowlinks,
        &mut on_stack,
        &mut stack,
        &mut result,
      );
    }
  }
  result
}

/// Structural-only walk collecting call-graph edges — every `Expr::
/// Call`/`Expr::CallKw`/`Expr::MethodCall` target that itself resolves
/// to another entry in `node_index` becomes an edge in `out`. Never
/// fails, never flags a violation (that's `check_purity_expr`'s job,
/// once the SCC processing order below is known) — this pass only
/// needs to know WHICH other `pure`-claimed nodes a body's calls can
/// reach, not whether any of them are legal.
fn collect_purity_edges_stmt(
  stmt: &Spanned<Stmt>,
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  out: &mut HashSet<usize>,
) {
  match &stmt.node {
    Stmt::Let { value, .. } | Stmt::SetField { value, .. } | Stmt::Assign { value, .. } => {
      collect_purity_edges_expr(value, node_index, env, classes, out)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      collect_purity_edges_expr(array, node_index, env, classes, out);
      collect_purity_edges_expr(index, node_index, env, classes, out);
      collect_purity_edges_expr(value, node_index, env, classes, out);
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        collect_purity_edges_expr(v, node_index, env, classes, out);
      }
    }
    Stmt::Return(Some(e)) | Stmt::Raise(e) => {
      collect_purity_edges_expr(e, node_index, env, classes, out)
    }
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Yield(args) => {
      for a in args {
        collect_purity_edges_expr(a, node_index, env, classes, out);
      }
    }
    Stmt::Expr(e) => collect_purity_edges_expr(e, node_index, env, classes, out),
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      collect_purity_edges_expr(cond, node_index, env, classes, out);
      for s in then_branch {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
      if let Some(eb) = else_branch {
        for s in eb {
          collect_purity_edges_stmt(s, node_index, env, classes, out);
        }
      }
    }
    Stmt::While { cond, body } => {
      collect_purity_edges_expr(cond, node_index, env, classes, out);
      for s in body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        collect_purity_edges_expr(e, node_index, env, classes, out);
      }
      for s in body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      collect_purity_edges_expr(start, node_index, env, classes, out);
      collect_purity_edges_expr(end, node_index, env, classes, out);
      for s in body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      collect_purity_edges_expr(scrutinee, node_index, env, classes, out);
      for (_, arm_body) in arms {
        for s in arm_body {
          collect_purity_edges_stmt(s, node_index, env, classes, out);
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          collect_purity_edges_stmt(s, node_index, env, classes, out);
        }
      }
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
      for r in rescues {
        for s in &r.body {
          collect_purity_edges_stmt(s, node_index, env, classes, out);
        }
      }
      if let Some(eb) = ensure {
        for s in eb {
          collect_purity_edges_stmt(s, node_index, env, classes, out);
        }
      }
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      collect_purity_edges_expr(scrutinee, node_index, env, classes, out);
      for s in ok_body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
      for s in err_body {
        collect_purity_edges_stmt(s, node_index, env, classes, out);
      }
    }
  }
}

/// `recv`'s callable owner name, resolved the same two ways
/// `check_purity_expr`'s own `MethodCall` arm resolves it: an
/// `Ident` bound in `env` to `Type::Class(n)` (an ordinary local/
/// param), or an `Ident` naming a module directly (modules are never
/// bound in `env` — there's no instance to bind). Shared by the edge
/// collector and the real checker so the two can never disagree about
/// which `MethodCall` sites are even candidates for a `pure`-relevant
/// dispatch.
fn purity_method_owner<'a>(
  recv: &Spanned<Expr>,
  env: &HashMap<String, Type>,
  classes: &'a HashMap<String, ClassInfo>,
) -> Option<&'a str> {
  let Expr::Ident(recv_name) = &recv.node else {
    return None;
  };
  if let Some(Type::Class(class_name)) = env.get(recv_name) {
    return classes.get_key_value(class_name).map(|(k, _)| k.as_str());
  }
  if classes.get(recv_name).is_some_and(|c| c.is_module) {
    return classes.get_key_value(recv_name).map(|(k, _)| k.as_str());
  }
  None
}

fn collect_purity_edges_expr(
  expr: &Spanned<Expr>,
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  out: &mut HashSet<usize>,
) {
  match &expr.node {
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::InstanceVar(_) => {}
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
      collect_purity_edges_expr(a, node_index, env, classes, out);
      collect_purity_edges_expr(b, node_index, env, classes, out);
    }
    Expr::Compare(a, _, b) | Expr::Coalesce(a, b) => {
      collect_purity_edges_expr(a, node_index, env, classes, out);
      collect_purity_edges_expr(b, node_index, env, classes, out);
    }
    Expr::Neg(a)
    | Expr::Not(a)
    | Expr::BitNot(a)
    | Expr::ArrayNew(a)
    | Expr::Ok(a)
    | Expr::Err(a)
    | Expr::Try(a)
    | Expr::Comptime(a) => collect_purity_edges_expr(a, node_index, env, classes, out),
    Expr::Call(name, args) => {
      if let Some(&idx) = node_index.get(&PureNode::Function(name.clone())) {
        out.insert(idx);
      }
      for a in args {
        collect_purity_edges_expr(a, node_index, env, classes, out);
      }
    }
    Expr::CallKw(name, kwargs) => {
      if let Some(&idx) = node_index.get(&PureNode::Function(name.clone())) {
        out.insert(idx);
      }
      for (_, v) in kwargs {
        collect_purity_edges_expr(v, node_index, env, classes, out);
      }
    }
    Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        collect_purity_edges_expr(a, node_index, env, classes, out);
      }
    }
    Expr::MethodCall(recv, method, args) | Expr::SafeCall(recv, method, args) => {
      if let Some(owner) = purity_method_owner(recv, env, classes) {
        if let Some(&idx) = node_index.get(&PureNode::Method(owner.to_string(), method.clone())) {
          out.insert(idx);
        }
      }
      collect_purity_edges_expr(recv, node_index, env, classes, out);
      // Plan 73: `Expr::Coalesce` handled at the tail of this match —
      // no purity edge of its own (neither operand is a method call
      // site), just recurses into both, mirroring `Compare`'s own
      // two-operand recursion below.
      for a in args {
        collect_purity_edges_expr(a, node_index, env, classes, out);
      }
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        collect_purity_edges_expr(e, node_index, env, classes, out);
      }
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        collect_purity_edges_expr(k, node_index, env, classes, out);
        collect_purity_edges_expr(v, node_index, env, classes, out);
      }
    }
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          collect_purity_edges_expr(e, node_index, env, classes, out);
        }
      }
    }
    // Mirrors `expr_moved_read`'s own identical `Lambda`/`Supervise`
    // arms (plan 56's Decision log) — a disclosed gap, not an
    // oversight: a lambda/`supervise` body is a nested `Stmt` list
    // this plan's own worked examples never put a `pure`-relevant call
    // inside.
    Expr::Lambda { .. } | Expr::Supervise(_) => {}
    Expr::Remote { addr, name, .. } => {
      collect_purity_edges_expr(addr, node_index, env, classes, out);
      collect_purity_edges_expr(name, node_index, env, classes, out);
    }
    Expr::Locate { key, args, .. } => {
      collect_purity_edges_expr(key, node_index, env, classes, out);
      for a in args {
        collect_purity_edges_expr(a, node_index, env, classes, out);
      }
    }
  }
}

/// `Expr::Call`/`Expr::CallKw`'s shared target-legality check (Decision
/// log, forbidden categories 1/3/5 for a *plain*-call callee — `puts`/
/// `gets`, an `extern "C"` declaration, and an ordinary/rejected
/// non-`pure` function, in that priority order). `cycle_members`/
/// `verified_pure` are exactly `check_purity`'s own per-component
/// state — a fellow member of the SCC currently being checked is
/// provisionally assumed pure (Decision log's cycle rule); anything
/// already accepted by an earlier-processed component is genuinely
/// pure; anything else that IS `pure`-claimed but not (yet) verified
/// has already failed (or belongs to a not-yet-reached component,
/// which cannot happen given `check_purity`'s reverse-topological
/// processing order).
fn check_purity_call_target(
  name: &str,
  span: (usize, usize),
  cycle_members: &HashSet<usize>,
  node_index: &HashMap<PureNode, usize>,
  sigs: &HashMap<String, FunctionSig>,
  extern_fn_names: &HashSet<String>,
  verified_pure: &HashSet<usize>,
) -> Result<(), Diagnostic> {
  if name == "puts" || name == "gets" {
    return Err(Diagnostic::new(
      format!("a `pure` function may not call `{name}` — it performs I/O"),
      span,
    ));
  }
  if extern_fn_names.contains(name) {
    return Err(Diagnostic::new(
      format!(
        "a `pure` function may not call `{name}` — it is an `extern \"C\"` FFI declaration, never provably pure"
      ),
      span,
    ));
  }
  // `FunctionSig.is_pure` (`function_signature`, mirroring `Function.
  // is_pure`) is the actual decision point here, not `node_index`
  // membership directly — every `pure`-claimed top-level function is
  // both a `sigs` entry with `is_pure: true` AND a `node_index`
  // candidate by construction (`check_purity`'s own candidate-
  // gathering loop), so the two never disagree; reading it straight
  // from `sigs` is what lets a call site resolve a callee's claim
  // without a second lookup back into the raw `Program` (Decision
  // log, forbidden category 5).
  if let Some(sig) = sigs.get(name) {
    if !sig.is_pure {
      return Err(Diagnostic::new(
        format!("a `pure` function may not call `{name}` — it is not marked `pure`"),
        span,
      ));
    }
    let idx = node_index[&PureNode::Function(name.to_string())];
    if cycle_members.contains(&idx) || verified_pure.contains(&idx) {
      return Ok(());
    }
    return Err(Diagnostic::new(
      format!("a `pure` function may not call `{name}` — it is not itself provably `pure`"),
      span,
    ));
  }
  // Every `Expr::Call` ordinary type-checking already accepted resolves
  // to a `sigs` entry — this arm is unreachable in practice, kept only
  // as a permissive (not panicking) fallback.
  Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_purity_stmt(
  stmt: &Spanned<Stmt>,
  cycle_members: &HashSet<usize>,
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  sigs: &HashMap<String, FunctionSig>,
  extern_fn_names: &HashSet<String>,
  verified_pure: &HashSet<usize>,
) -> Result<(), Diagnostic> {
  match &stmt.node {
    // Decision log, forbidden category 4 — the only two mutation
    // mechanisms this whole compiler has, both forbidden outright.
    Stmt::SetField { .. } => Err(Diagnostic::new(
      "a `pure` function may not write a field (`@x = ...`) — field mutation is forbidden inside a `pure` function",
      stmt.span,
    )),
    Stmt::SetIndex { .. } => Err(Diagnostic::new(
      "a `pure` function may not write an index (`arr[i] = ...`) — index mutation is forbidden inside a `pure` function",
      stmt.span,
    )),
    Stmt::Let { value, .. } | Stmt::Assign { value, .. } => check_purity_expr(
      value,
      cycle_members,
      node_index,
      env,
      classes,
      sigs,
      extern_fn_names,
      verified_pure,
    ),
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        check_purity_expr(
          v,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Stmt::Return(Some(e)) | Stmt::Raise(e) => check_purity_expr(
      e,
      cycle_members,
      node_index,
      env,
      classes,
      sigs,
      extern_fn_names,
      verified_pure,
    ),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => Ok(()),
    Stmt::Yield(args) => {
      for a in args {
        check_purity_expr(
          a,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Stmt::Expr(e) => check_purity_expr(
      e,
      cycle_members,
      node_index,
      env,
      classes,
      sigs,
      extern_fn_names,
      verified_pure,
    ),
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      check_purity_expr(
        cond,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for s in then_branch {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      if let Some(eb) = else_branch {
        for s in eb {
          check_purity_stmt(
            s,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      check_purity_expr(
        cond,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for s in body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        check_purity_expr(
          e,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      for s in body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      check_purity_expr(
        start,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      check_purity_expr(
        end,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for s in body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      check_purity_expr(
        scrutinee,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for (_, arm_body) in arms {
        for s in arm_body {
          check_purity_stmt(
            s,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          check_purity_stmt(
            s,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      Ok(())
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      for r in rescues {
        for s in &r.body {
          check_purity_stmt(
            s,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      if let Some(eb) = ensure {
        for s in eb {
          check_purity_stmt(
            s,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      Ok(())
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      check_purity_expr(
        scrutinee,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for s in ok_body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      for s in err_body {
        check_purity_stmt(
          s,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
  }
}

#[allow(clippy::too_many_arguments)]
fn check_purity_expr(
  expr: &Spanned<Expr>,
  cycle_members: &HashSet<usize>,
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  sigs: &HashMap<String, FunctionSig>,
  extern_fn_names: &HashSet<String>,
  verified_pure: &HashSet<usize>,
) -> Result<(), Diagnostic> {
  match &expr.node {
    // Decision log, forbidden category 1: `ARGV`/`ARGC` are ordinary
    // pre-seeded locals at codegen time, not literals — nothing about
    // a `pure` function's own *type* distinguishes "reads `ARGV`" from
    // "reads a genuinely constant global," so this closes the gap by
    // name.
    Expr::Ident(name) if name == "ARGV" || name == "ARGC" => Err(Diagnostic::new(
      format!(
        "a `pure` function may not read `{name}` — its value can vary with how the compiled binary was invoked"
      ),
      expr.span,
    )),
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::InstanceVar(_) => Ok(()),
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
      check_purity_expr(
        a,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      check_purity_expr(
        b,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )
    }
    Expr::Compare(a, _, b) | Expr::Coalesce(a, b) => {
      check_purity_expr(
        a,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      check_purity_expr(
        b,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )
    }
    Expr::Neg(a)
    | Expr::Not(a)
    | Expr::BitNot(a)
    | Expr::ArrayNew(a)
    | Expr::Ok(a)
    | Expr::Err(a)
    | Expr::Try(a)
    | Expr::Comptime(a) => check_purity_expr(
      a,
      cycle_members,
      node_index,
      env,
      classes,
      sigs,
      extern_fn_names,
      verified_pure,
    ),
    Expr::Call(name, args) => {
      check_purity_call_target(
        name,
        expr.span,
        cycle_members,
        node_index,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for a in args {
        check_purity_expr(
          a,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Expr::CallKw(name, kwargs) => {
      check_purity_call_target(
        name,
        expr.span,
        cycle_members,
        node_index,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for (_, v) in kwargs {
        check_purity_expr(
          v,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    // `.new(...)` is "trivially fresh" the same way plan 56's own
    // `check_message_arg` bucket 2 already treats it — allocating a
    // new instance isn't itself a forbidden construct; only `.spawn`
    // (below) is.
    Expr::New(_, args) => {
      for a in args {
        check_purity_expr(
          a,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    // Decision log, forbidden category 2 — unconditional, no receiver
    // check needed (there's nothing conditional about creating an
    // actor).
    Expr::Spawn(_, _) => Err(Diagnostic::new(
      "a `pure` function may not call `.spawn` — creating a new actor is forbidden inside a `pure` function",
      expr.span,
    )),
    Expr::MethodCall(recv, method, args) => {
      if let Expr::Ident(recv_name) = &recv.node {
        if recv_name == "File" {
          // Decision log, forbidden category 1 — `File.read`/`File.
          // write`, `Expr::MethodCall` whose receiver is literally
          // `Expr::Ident("File")` (plan 45).
          return Err(Diagnostic::new(
            format!("a `pure` function may not call `File.{method}` — it performs I/O"),
            expr.span,
          ));
        }
      }
      if let Some(owner) = purity_method_owner(recv, env, classes) {
        if classes.get(owner).is_some_and(|c| c.is_actor) {
          // Decision log, forbidden category 2 — a cross-actor
          // message send (`pure` is never legal on an actor method
          // itself, so every receiver this arm can ever see is a
          // genuinely different actor, never `self`).
          return Err(Diagnostic::new(
            format!(
              "a `pure` function may not send a message to an actor (`{owner}.{method}(...)`) — this is a message send, forbidden inside a `pure` function"
            ),
            expr.span,
          ));
        }
        // `FunctionSig.is_pure` here too (`ClassInfo.methods`'s own
        // `HashMap<String, FunctionSig>`, `build_flattened_class_info`)
        // — the exact same lookup a bare top-level call already uses
        // (Decision log, forbidden category 5's own stated design),
        // not a second, `node_index`-only decision.
        if let Some(method_sig) = classes.get(owner).and_then(|c| c.methods.get(method)) {
          if !method_sig.is_pure {
            return Err(Diagnostic::new(
              format!(
                "a `pure` function may not call `{owner}#{method}` — it is not marked `pure`"
              ),
              expr.span,
            ));
          }
          let target = PureNode::Method(owner.to_string(), method.clone());
          let idx = node_index[&target];
          if !cycle_members.contains(&idx) && !verified_pure.contains(&idx) {
            return Err(Diagnostic::new(
              format!(
                "a `pure` function may not call `{owner}#{method}` — it is not itself provably `pure`"
              ),
              expr.span,
            ));
          }
        }
      }
      check_purity_expr(
        recv,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for a in args {
        check_purity_expr(
          a,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    // Plan 43: only ever legal on a nullable receiver — never an
    // actor-typed one (`env`'s `Type::Class` lookup above is the only
    // path to an actor receiver at all), so no cross-actor-send check
    // applies here.
    Expr::SafeCall(recv, _method, args) => {
      check_purity_expr(
        recv,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      for a in args {
        check_purity_expr(
          a,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        check_purity_expr(
          e,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        check_purity_expr(
          k,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
        check_purity_expr(
          v,
          cycle_members,
          node_index,
          env,
          classes,
          sigs,
          extern_fn_names,
          verified_pure,
        )?;
      }
      Ok(())
    }
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          check_purity_expr(
            e,
            cycle_members,
            node_index,
            env,
            classes,
            sigs,
            extern_fn_names,
            verified_pure,
          )?;
        }
      }
      Ok(())
    }
    // Mirrors `expr_moved_read`'s own identical `Lambda`/`Supervise`
    // arms (plan 56's Decision log) — same disclosed gap, cited above
    // in `collect_purity_edges_expr`.
    Expr::Lambda { .. } | Expr::Supervise(_) => Ok(()),
    Expr::Remote { addr, name, .. } => {
      check_purity_expr(
        addr,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )?;
      check_purity_expr(
        name,
        cycle_members,
        node_index,
        env,
        classes,
        sigs,
        extern_fn_names,
        verified_pure,
      )
    }
    // Plan 65's Decision log: `.locate` may lazily spawn a fresh actor
    // instance (running a real `initialize`) exactly like `.spawn`
    // itself, or perform a remote connect — at least as much of a
    // side effect as `.spawn`'s own unconditional forbid above, so it
    // gets the identical treatment.
    Expr::Locate { .. } => Err(Diagnostic::new(
      "a `pure` function may not call `.locate` — activating an actor is forbidden inside a `pure` function",
      expr.span,
    )),
  }
}

/// Plan 63's `leaf-sema-purity-check`: `pure` is a real, whole-program
/// checked property, not a naming convention (Decision log) — this is
/// the only place `Function.is_pure`/`FunctionSig.is_pure` are ever
/// consulted beyond straight field-copying. Runs once from
/// `check_program`, after every `check_function_body`/`check_method_
/// body` call has already run for the whole program (mirrors exactly
/// when plan 56's `check_message_safety` already runs relative to
/// ordinary type-checking) — `sigs`/`classes` are the finished,
/// trustworthy registries `check_program` built once at its own top,
/// never mutated here. A generic class's own monomorphized
/// instantiations (plan 58) are a disclosed gap: they're never
/// literal `program.items` entries, so a `pure` method declared on a
/// generic class template is registered as a candidate (via its
/// template `Item::Class`) but its actually-instantiated, substituted
/// bodies are not separately re-verified here — the same scope plan
/// 61's `comptime` similarly never extended to generic instantiation.
fn check_purity(
  program: &Program,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Vec<Diagnostic>> {
  let mut diags = Vec::new();

  // Decision log, forbidden category 3 — collected the same way plan
  // 61's `comptime_fns` is, so `check_purity_call_target` can name an
  // extern call specifically rather than folding it into the generic
  // "not marked `pure`" message every other ordinary function gets.
  let extern_fn_names: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Extern(block) => Some(block.fns.iter().map(|f| f.name.clone())),
      _ => None,
    })
    .flatten()
    .collect();

  let mut candidates: Vec<PureCandidate> = Vec::new();
  for item in &program.items {
    match item {
      Item::Function(f) if f.is_pure => {
        if !f.type_params.is_empty() {
          diags.push(Diagnostic::new(
            format!(
              "generic functions cannot be marked `pure` yet (`{}`)",
              f.name
            ),
            (0, 0),
          ));
          continue;
        }
        candidates.push(PureCandidate {
          node: PureNode::Function(f.name.clone()),
          f,
          self_fields: None,
        });
      }
      Item::Module(m) => {
        for f in &m.methods {
          if f.is_pure {
            candidates.push(PureCandidate {
              node: PureNode::Method(m.name.clone(), f.name.clone()),
              f,
              self_fields: None,
            });
          }
        }
      }
      Item::Class(c) => {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        for m in &c.methods {
          if m.is_pure {
            candidates.push(PureCandidate {
              node: PureNode::Method(c.name.clone(), m.name.clone()),
              f: m,
              self_fields: Some(&info.fields),
            });
          }
        }
      }
      // Decision log's own eligibility rule: `pure` is rejected on an
      // actor method outright, "before doing anything else" — no
      // candidate is ever built for it, and this pass returns as soon
      // as this loop finishes if any such diagnostic was raised (AC8:
      // independent of what that method's body contains).
      Item::Actor(a) => {
        for m in &a.methods {
          if m.is_pure {
            diags.push(Diagnostic::new(
              format!(
                "`pure` is not supported on an actor method (`{}#{}`) — an actor's own fields are already protected by its single-writer mailbox discipline, a different mechanism `pure` does not layer its checked guarantee on top of",
                a.name, m.name
              ),
              (0, 0),
            ));
          }
        }
      }
      _ => {}
    }
  }
  if !diags.is_empty() {
    return Err(diags);
  }
  if candidates.is_empty() {
    return Ok(());
  }

  let node_index: HashMap<PureNode, usize> = candidates
    .iter()
    .enumerate()
    .map(|(i, c)| (c.node.clone(), i))
    .collect();

  let envs: Vec<(HashMap<String, Type>, Type)> = candidates
    .iter()
    .map(|c| rebuild_purity_env(c.f, c.self_fields, sigs, classes, gctx))
    .collect();

  let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); candidates.len()];
  for (i, c) in candidates.iter().enumerate() {
    let mut edges = HashSet::new();
    collect_purity_edges(&c.f.body, &node_index, &envs[i].0, classes, &mut edges);
    adjacency[i] = edges.into_iter().collect();
  }

  let components = tarjan_scc(&adjacency);
  let mut verified_pure: HashSet<usize> = HashSet::new();

  for component in &components {
    let cycle_members: HashSet<usize> = component.iter().copied().collect();
    let results: Vec<(usize, Result<(), Diagnostic>)> = component
      .iter()
      .map(|&idx| {
        let c = &candidates[idx];
        let r = check_purity_body(
          &c.f.body,
          &cycle_members,
          &node_index,
          &envs[idx].0,
          classes,
          sigs,
          &extern_fn_names,
          &verified_pure,
        );
        (idx, r)
      })
      .collect();
    let component_ok = results.iter().all(|(_, r)| r.is_ok());
    if component_ok {
      for &idx in component {
        verified_pure.insert(idx);
      }
    } else {
      for (idx, r) in results {
        match r {
          Err(d) => diags.push(d),
          // This member has no violation of its own — it's rejected
          // solely because a fellow member of its mutually-recursive
          // (or self-recursive-and-otherwise-clean, impossible here
          // since a lone clean self-recursive node always passes on
          // its own) SCC failed (Decision log's cycle rule: the whole
          // component stands or falls together).
          Ok(()) => diags.push(Diagnostic::new(
            format!(
              "`{}` cannot be verified `pure` — it belongs to a mutually-recursive purity group with a member that is not itself provably `pure`",
              pure_node_display_name(&candidates[idx].node)
            ),
            (0, 0),
          )),
        }
      }
    }
  }

  if diags.is_empty() {
    Ok(())
  } else {
    Err(diags)
  }
}

fn collect_purity_edges(
  body: &[Spanned<Stmt>],
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  out: &mut HashSet<usize>,
) {
  for stmt in body {
    collect_purity_edges_stmt(stmt, node_index, env, classes, out);
  }
}

#[allow(clippy::too_many_arguments)]
fn check_purity_body(
  body: &[Spanned<Stmt>],
  cycle_members: &HashSet<usize>,
  node_index: &HashMap<PureNode, usize>,
  env: &HashMap<String, Type>,
  classes: &HashMap<String, ClassInfo>,
  sigs: &HashMap<String, FunctionSig>,
  extern_fn_names: &HashSet<String>,
  verified_pure: &HashSet<usize>,
) -> Result<(), Diagnostic> {
  for stmt in body {
    check_purity_stmt(
      stmt,
      cycle_members,
      node_index,
      env,
      classes,
      sigs,
      extern_fn_names,
      verified_pure,
    )?;
  }
  Ok(())
}

/// Plan 61's Decision log: a concrete, enumerated ALLOW-list, not an
/// implicit "everything except the ban-list" — a node kind absent from
/// both the allow-list and the ban-list below is still rejected (via
/// the final catch-all arms), just with a generic message rather than
/// one naming the specific disallowed construct. Purely structural —
/// walks the AST shape only, never evaluates anything, so (unlike the
/// interpreter itself) this always terminates: no step ceiling needed
/// here, only in `emerald-codegen`'s `ComptimeInterpreter::eval`.
/// `comptime_fns` is every top-level function's own name known to be
/// `is_comptime: true` — `Call`'s own arm is what makes this
/// compositional (a `comptime` function may call another `comptime`
/// function, itself independently checked against this exact same
/// allow-list when `check_program` reaches its own top-level
/// registration, never an ordinary one).
fn check_comptime_legal(
  body: &[Spanned<Stmt>],
  comptime_fns: &HashSet<String>,
) -> Result<(), Diagnostic> {
  for stmt in body {
    check_comptime_legal_stmt(stmt, comptime_fns)?;
  }
  Ok(())
}

fn check_comptime_legal_stmt(
  stmt: &Spanned<Stmt>,
  comptime_fns: &HashSet<String>,
) -> Result<(), Diagnostic> {
  match &stmt.node {
    Stmt::Let { value, .. } | Stmt::Assign { value, .. } => {
      check_comptime_legal_expr(value, comptime_fns)
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        check_comptime_legal_expr(v, comptime_fns)?;
      }
      Ok(())
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      check_comptime_legal_expr(cond, comptime_fns)?;
      check_comptime_legal(then_branch, comptime_fns)?;
      if let Some(eb) = else_branch {
        check_comptime_legal(eb, comptime_fns)?;
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      check_comptime_legal_expr(cond, comptime_fns)?;
      check_comptime_legal(body, comptime_fns)
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        check_comptime_legal_expr(e, comptime_fns)?;
      }
      check_comptime_legal(body, comptime_fns)
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      check_comptime_legal_expr(start, comptime_fns)?;
      check_comptime_legal_expr(end, comptime_fns)?;
      check_comptime_legal(body, comptime_fns)
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      check_comptime_legal_expr(scrutinee, comptime_fns)?;
      for (pat, arm_body) in arms {
        if let CasePattern::Values(vs) = pat {
          for v in vs {
            check_comptime_legal_expr(v, comptime_fns)?;
          }
        }
        check_comptime_legal(arm_body, comptime_fns)?;
      }
      if let Some(eb) = else_body {
        check_comptime_legal(eb, comptime_fns)?;
      }
      Ok(())
    }
    Stmt::Return(Some(e)) => check_comptime_legal_expr(e, comptime_fns),
    Stmt::Return(None) => Ok(()),
    Stmt::Expr(e) => check_comptime_legal_expr(e, comptime_fns),
    Stmt::Begin { .. } | Stmt::Raise(_) | Stmt::Retry => Err(Diagnostic::new(
      "comptime evaluation may not `raise`/`rescue` — exception unwinding is not modeled by the compile-time interpreter",
      stmt.span,
    )),
    Stmt::Yield(_) => Err(Diagnostic::new(
      "comptime evaluation may not construct a lambda — closures capturing runtime state have no compile-time meaning",
      stmt.span,
    )),
    Stmt::MatchResult { .. } => Err(Diagnostic::new(
      "comptime evaluation may not use `Result[T, E]` — Result unwinding is not modeled by the compile-time interpreter",
      stmt.span,
    )),
    Stmt::SetField { .. } | Stmt::SetIndex { .. } | Stmt::Break | Stmt::Next => Err(Diagnostic::new(
      "this statement form is not part of the comptime-legal subset",
      stmt.span,
    )),
  }
}

fn check_comptime_legal_expr(
  expr: &Spanned<Expr>,
  comptime_fns: &HashSet<String>,
) -> Result<(), Diagnostic> {
  match &expr.node {
    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::InstanceVar(_) => Ok(()),
    // `ARGV`/`ARGC` (plan 45) are the only process-environment-dependent
    // bare identifiers this stdlib surface has — every other `Ident` is
    // an ordinary local/parameter read, legal.
    Expr::Ident(name) if name == "ARGV" || name == "ARGC" => Err(Diagnostic::new(
      format!(
        "comptime evaluation may not reference `{name}` — I/O is not available at compile time"
      ),
      expr.span,
    )),
    Expr::Ident(_) => Ok(()),
    Expr::Add(a, b)
    | Expr::Sub(a, b)
    | Expr::Mul(a, b)
    | Expr::Rem(a, b)
    | Expr::Div(a, b)
    | Expr::And(a, b)
    | Expr::Or(a, b)
    | Expr::BitAnd(a, b)
    | Expr::BitOr(a, b)
    | Expr::BitXor(a, b)
    | Expr::Shl(a, b)
    | Expr::Shr(a, b) => {
      check_comptime_legal_expr(a, comptime_fns)?;
      check_comptime_legal_expr(b, comptime_fns)
    }
    Expr::Compare(a, _, b) => {
      check_comptime_legal_expr(a, comptime_fns)?;
      check_comptime_legal_expr(b, comptime_fns)
    }
    Expr::Neg(a) | Expr::Not(a) | Expr::BitNot(a) => check_comptime_legal_expr(a, comptime_fns),
    Expr::New(_, args) => {
      for a in args {
        check_comptime_legal_expr(a, comptime_fns)?;
      }
      Ok(())
    }
    Expr::Call(name, args) if name == "puts" || name == "gets" => Err(Diagnostic::new(
      format!("comptime evaluation may not call `{name}` — I/O is not available at compile time"),
      expr.span,
    )),
    Expr::Call(name, args) if !comptime_fns.contains(name) => Err(Diagnostic::new(
      format!(
        "comptime evaluation may not call `{name}` — mark it `def comptime {name}(...)` if its body is a legal comptime subset"
      ),
      expr.span,
    )),
    Expr::Call(_, args) => {
      for a in args {
        check_comptime_legal_expr(a, comptime_fns)?;
      }
      Ok(())
    }
    Expr::MethodCall(recv, method, _)
      if matches!(&recv.node, Expr::Ident(r) if r == "File")
        && (method == "read" || method == "write") =>
    {
      Err(Diagnostic::new(
        format!(
          "comptime evaluation may not call `File.{method}` — I/O is not available at compile time"
        ),
        expr.span,
      ))
    }
    Expr::Spawn(_, _) => Err(Diagnostic::new(
      "comptime evaluation may not `.spawn` — actor isolation and message delivery are runtime concepts",
      expr.span,
    )),
    Expr::Supervise(_) => Err(Diagnostic::new(
      "comptime evaluation may not `supervise` — actor isolation and message delivery are runtime concepts",
      expr.span,
    )),
    Expr::Lambda { .. } => Err(Diagnostic::new(
      "comptime evaluation may not construct a lambda — closures capturing runtime state have no compile-time meaning",
      expr.span,
    )),
    Expr::Ok(_) | Expr::Err(_) | Expr::Try(_) => Err(Diagnostic::new(
      "comptime evaluation may not use `Result[T, E]` — Result unwinding is not modeled by the compile-time interpreter",
      expr.span,
    )),
    Expr::StringLit(_)
    | Expr::Interpolate(_)
    | Expr::SymbolLit(_)
    | Expr::CallKw(_, _)
    | Expr::MethodCall(_, _, _)
    | Expr::SafeCall(_, _, _)
    | Expr::Coalesce(_, _)
    | Expr::ArrayLit(_)
    | Expr::Index(_, _)
    | Expr::HashLit(_)
    | Expr::ArrayNew(_)
    | Expr::TupleLit(_)
    | Expr::Remote { .. }
    | Expr::Locate { .. }
    | Expr::Comptime(_) => Err(Diagnostic::new(
      "this expression form is not part of the comptime-legal subset",
      expr.span,
    )),
  }
}

/// Plan 61's Decision log: `Expr::Comptime`'s own legal *position*
/// restriction — exactly a top-level `Stmt::Let`'s direct value, or
/// `Expr::ArrayNew`'s direct size argument — enforced by a standalone
/// walk over the whole `Program`, the same "separate pass, not woven
/// into `check_stmt`'s shared plumbing" precedent `check_message_safety`/
/// `check_block_call_sites` already establish (`check_stmt`/`check_
/// block` have no "am I at the top level" context to thread without an
/// invasive signature change touching every call site).
const COMPTIME_POSITION_MESSAGE: &str =
  "`comptime` may only appear as a top-level constant's initializer or `Array.new`'s size argument";

fn check_comptime_positions(program: &Program) -> Vec<Diagnostic> {
  let mut diags = Vec::new();
  for item in &program.items {
    match item {
      Item::Stmt(s) => scan_comptime_position_stmt(s, &mut diags, true),
      Item::Function(f) => {
        for s in &f.body {
          scan_comptime_position_stmt(s, &mut diags, false);
        }
      }
      Item::Class(c) => {
        for m in &c.methods {
          for s in &m.body {
            scan_comptime_position_stmt(s, &mut diags, false);
          }
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          for s in &f.body {
            scan_comptime_position_stmt(s, &mut diags, false);
          }
        }
      }
      Item::Actor(a) => {
        for m in &a.methods {
          for s in &m.body {
            scan_comptime_position_stmt(s, &mut diags, false);
          }
        }
      }
      _ => {}
    }
  }
  diags
}

fn scan_comptime_position_stmt(stmt: &Spanned<Stmt>, diags: &mut Vec<Diagnostic>, top_level: bool) {
  match &stmt.node {
    Stmt::Let { value, .. } => {
      if top_level {
        if let Expr::Comptime(inner) = &value.node {
          scan_comptime_position_expr(inner, diags);
          return;
        }
      }
      scan_comptime_position_expr(value, diags);
    }
    Stmt::SetField { value, .. } | Stmt::Assign { value, .. } => {
      scan_comptime_position_expr(value, diags)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      scan_comptime_position_expr(array, diags);
      scan_comptime_position_expr(index, diags);
      scan_comptime_position_expr(value, diags);
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        scan_comptime_position_expr(v, diags);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      scan_comptime_position_expr(cond, diags);
      for s in then_branch {
        scan_comptime_position_stmt(s, diags, false);
      }
      if let Some(eb) = else_branch {
        for s in eb {
          scan_comptime_position_stmt(s, diags, false);
        }
      }
    }
    Stmt::While { cond, body } => {
      scan_comptime_position_expr(cond, diags);
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
    Stmt::Return(Some(e)) => scan_comptime_position_expr(e, diags),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Expr(e) => scan_comptime_position_expr(e, diags),
    Stmt::Raise(e) => scan_comptime_position_expr(e, diags),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
      for r in rescues {
        for s in &r.body {
          scan_comptime_position_stmt(s, diags, false);
        }
      }
      if let Some(en) = ensure {
        for s in en {
          scan_comptime_position_stmt(s, diags, false);
        }
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      scan_comptime_position_expr(scrutinee, diags);
      for (pat, body) in arms {
        if let CasePattern::Values(vs) = pat {
          for v in vs {
            scan_comptime_position_expr(v, diags);
          }
        }
        for s in body {
          scan_comptime_position_stmt(s, diags, false);
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          scan_comptime_position_stmt(s, diags, false);
        }
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        scan_comptime_position_expr(e, diags);
      }
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
    Stmt::Yield(args) => {
      for a in args {
        scan_comptime_position_expr(a, diags);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      scan_comptime_position_expr(start, diags);
      scan_comptime_position_expr(end, diags);
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      scan_comptime_position_expr(scrutinee, diags);
      for s in ok_body {
        scan_comptime_position_stmt(s, diags, false);
      }
      for s in err_body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
  }
}

fn scan_comptime_position_expr(expr: &Spanned<Expr>, diags: &mut Vec<Diagnostic>) {
  match &expr.node {
    Expr::Comptime(inner) => {
      diags.push(Diagnostic::new(COMPTIME_POSITION_MESSAGE, expr.span));
      scan_comptime_position_expr(inner, diags);
    }
    Expr::ArrayNew(size) => {
      if let Expr::Comptime(inner) = &size.node {
        scan_comptime_position_expr(inner, diags);
      } else {
        scan_comptime_position_expr(size, diags);
      }
    }
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::InstanceVar(_)
    | Expr::Bool(_) => {}
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          scan_comptime_position_expr(e, diags);
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
      scan_comptime_position_expr(a, diags);
      scan_comptime_position_expr(b, diags);
    }
    Expr::Neg(a) | Expr::Not(a) | Expr::BitNot(a) | Expr::Ok(a) | Expr::Err(a) | Expr::Try(a) => {
      scan_comptime_position_expr(a, diags);
    }
    Expr::Compare(a, _, b) => {
      scan_comptime_position_expr(a, diags);
      scan_comptime_position_expr(b, diags);
    }
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        scan_comptime_position_expr(a, diags);
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        scan_comptime_position_expr(v, diags);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      scan_comptime_position_expr(recv, diags);
      for a in args {
        scan_comptime_position_expr(a, diags);
      }
    }
    Expr::Coalesce(a, b) => {
      scan_comptime_position_expr(a, diags);
      scan_comptime_position_expr(b, diags);
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        scan_comptime_position_expr(e, diags);
      }
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        scan_comptime_position_expr(k, diags);
        scan_comptime_position_expr(v, diags);
      }
    }
    Expr::Lambda { body, .. } => {
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
    Expr::Supervise(body) => {
      for s in body {
        scan_comptime_position_stmt(s, diags, false);
      }
    }
    Expr::Remote { addr, name, .. } => {
      scan_comptime_position_expr(addr, diags);
      scan_comptime_position_expr(name, diags);
    }
    Expr::Locate { key, args, .. } => {
      scan_comptime_position_expr(key, diags);
      for a in args {
        scan_comptime_position_expr(a, diags);
      }
    }
  }
}

/// Plan 62's Decision log: `f.requires[i].expr` type-checks against the
/// existing, unmodified params-only `env` (`requires` is a property of
/// the arguments a caller supplies); `f.ensures[i].expr` type-checks
/// against a *clone* of that same `env` with one extra entry, `"result"
/// -> declared_return`, inserted first — no grammar/AST support needed
/// for the pseudo-identifier `result` at all, it's an ordinary `Expr::
/// Ident` given meaning only by this one extra `env` entry. Both require
/// `Type::Boolean` back via the existing, unmodified `infer_expr_type`.
fn check_contracts(
  f: &Function,
  env: &HashMap<String, Type>,
  declared_return: &Type,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  for c in &f.requires {
    let ty = infer_expr_type(&c.expr, env, sigs, classes, None, gctx)?;
    if ty != Type::Boolean {
      return Err(Diagnostic::new(
        format!(
          "`{}`'s `requires {}` must be Boolean, found {ty:?}",
          f.name, c.text
        ),
        c.expr.span,
      ));
    }
  }
  if !f.ensures.is_empty() {
    let mut ensures_env = env.clone();
    ensures_env.insert("result".to_string(), declared_return.clone());
    for c in &f.ensures {
      let ty = infer_expr_type(&c.expr, &ensures_env, sigs, classes, None, gctx)?;
      if ty != Type::Boolean {
        return Err(Diagnostic::new(
          format!(
            "`{}`'s `ensures {}` must be Boolean, found {ty:?}",
            f.name, c.text
          ),
          c.expr.span,
        ));
      }
    }
  }
  Ok(())
}

fn check_function_body(
  f: &Function,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  // Plan 72's Decision log: a function parameter has no `var` slot
  // anywhere in this grammar (`ParenParams`/`Params` never accept one)
  // — an ordinary `env.insert`, never `declare_local`, so every
  // parameter is unconditionally immutable inside its own function
  // body. This is the simplest rule available (no syntax exists to opt
  // a parameter into mutability, so there's no "sometimes" case to
  // design around) and matches Sable's own examples, none of which
  // reassign a parameter.
  let mut mutable_locals: HashSet<String> = HashSet::new();
  for p in &f.params {
    env.insert(p.name.clone(), resolve_type(&p.ty, classes)?);
  }
  // Plan 39: a splat parameter is bound inside the body as a real
  // `Array[Elem]` — call sites pack it into one at each call site (the
  // same representation plan 09 already proved), so the body indexes
  // it exactly like any other array-typed local. Plan 72: same
  // immutable-by-construction rule as an ordinary parameter above.
  if let Some(p) = &f.splat_param {
    let elem_ty = resolve_type(&p.ty, classes)?;
    env.insert(p.name.clone(), Type::Array(Box::new(elem_ty)));
  }
  let declared_return = resolve_return_type(&f.return_type, classes)?;
  check_contracts(f, &env, &declared_return, sigs, classes, gctx)?;
  check_block(
    &f.body,
    &mut env,
    &mut mutable_locals,
    sigs,
    classes,
    None,
    &declared_return,
    false,
    false,
    f.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &f.body,
    &env,
    sigs,
    classes,
    None,
    &declared_return,
    &f.name,
    gctx,
  )?;
  // Plan 56 (compile-time message safety) — run once the body's own
  // ordinary type-check has already succeeded, reusing its final `env`
  // read-only.
  let mut moved = HashMap::new();
  check_message_safety(&f.body, &env, classes, &mut moved)
}

fn check_method_body(
  class_name: &str,
  m: &Function,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  fields: &HashMap<String, Type>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  // Plan 39's Decision log: default parameter values and splat capture
  // are scoped to plain top-level `def` functions only — a real,
  // disclosed diagnostic here, not a silently-ignored parsed-but-dead
  // AST field.
  if let Some(p) = m.params.iter().find(|p| p.default.is_some()) {
    return Err(Diagnostic::new(
      format!(
        "default parameter values are not supported on methods yet (`{class_name}#{}`'s `{}`)",
        m.name, p.name
      ),
      (0, 0),
    ));
  }
  if m.splat_param.is_some() {
    return Err(Diagnostic::new(
      format!(
        "splat parameters are not supported on methods yet (`{class_name}#{}`)",
        m.name
      ),
      (0, 0),
    ));
  }
  // Plan 39's Decision log: a tuple return type is scoped to plain
  // top-level `def` functions too — checked here, before the ordinary
  // (non-tuple-aware) `resolve_type` call below, which would otherwise
  // reject this shape with a generic "unknown type" diagnostic instead
  // of this explicit, named one.
  if matches!(m.return_type, TypeExpr::Tuple(_)) {
    return Err(Diagnostic::new(
      format!(
        "tuple return types are not supported on methods yet (`{class_name}#{}`)",
        m.name
      ),
      (0, 0),
    ));
  }
  // Plan 62's Decision log: defense in depth — `requires`/`ensures` are
  // already grammatically unreachable on a `MethodDef` (no `ContractClause*`
  // slot exists in that production at all), so this can only ever fire
  // against a hand-constructed AST, not real source text. Mirrors the
  // three checks immediately above exactly.
  if !m.requires.is_empty() || !m.ensures.is_empty() {
    return Err(Diagnostic::new(
      format!(
        "contracts are not supported on methods yet (`{class_name}#{}`)",
        m.name
      ),
      (0, 0),
    ));
  }
  let mut env = HashMap::new();
  // Plan 72's Decision log: same as `check_function_body` — a method
  // parameter is unconditionally immutable inside its own body.
  let mut mutable_locals: HashSet<String> = HashSet::new();
  for p in &m.params {
    env.insert(p.name.clone(), resolve_type(&p.ty, classes)?);
  }
  let declared_return = resolve_type(&m.return_type, classes)?;
  check_block(
    &m.body,
    &mut env,
    &mut mutable_locals,
    sigs,
    classes,
    Some(fields),
    &declared_return,
    false,
    false,
    m.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &m.body,
    &env,
    sigs,
    classes,
    Some(fields),
    &declared_return,
    &format!("{class_name}#{}", m.name),
    gctx,
  )?;
  // Plan 56 (compile-time message safety) — see `check_function_body`'s
  // identical call for the full rationale; a method body (including an
  // actor's own method sending to ANOTHER actor) gets the exact same
  // check.
  let mut moved = HashMap::new();
  check_message_safety(&m.body, &env, classes, &mut moved)
}

/// Plan 34: for every bare top-level statement call to a `block_param`-
/// declaring free function (`Stmt::Expr(Expr::Call(...))` — the only
/// shape a trailing block literal can attach to, since `BlockLiteral`
/// is grammar-restricted to `StmtPrimaryExpr`, never the non-Stmt-
/// initial `PrimaryExpr` — see `grammar.lalrpop`'s Decision log),
/// checks that a block is actually attached, type-checks that block's
/// own body, and then re-walks the callee's OWN body checking every
/// `Stmt::Yield` site's arguments against this specific attachment's
/// parameter types (Decision log: there is no single fixed block
/// signature to check against once — two call sites attaching
/// different blocks to the same function are each checked
/// independently). Scoped to `Expr::Call` only, not `MethodCall` — this
/// plan's own worked example and ACs only exercise a free function; a
/// `block_param`-declaring method is real, disclosed future work.
fn check_block_call_sites(
  program: &Program,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
) -> Vec<Diagnostic> {
  let mut diags = Vec::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        scan_block_call_sites(&f.body, sigs, classes, func_defs, gctx, &mut diags)
      }
      Item::Class(c) => {
        for m in &c.methods {
          scan_block_call_sites(&m.body, sigs, classes, func_defs, gctx, &mut diags);
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          scan_block_call_sites(&f.body, sigs, classes, func_defs, gctx, &mut diags);
        }
      }
      Item::Stmt(s) => scan_block_call_site(s, sigs, classes, func_defs, gctx, &mut diags),
      // Plan 41: an interface declares one required method signature,
      // never a body — nothing here can contain a `yield` site.
      Item::Interface(_) => {}
      // Plan 17/46: `emerald-cli`'s own `require.rs` (not
      // `emerald-driver`) splices multi-file `require`s before a
      // `Program` ever reaches `emerald-sema` — a no-op here, not an
      // error, since a `require`-bearing `Program` reaching this far
      // is a real, disclosed gap this plan names rather than papering
      // over.
      Item::Require(_) => {}
      // Plan 47: a `test` body is checked exactly like a free
      // function's — it can contain a block-attaching call site too.
      Item::Test { body, .. } => {
        scan_block_call_sites(body, sigs, classes, func_defs, gctx, &mut diags)
      }
      // Plan 52: an enum is pure data — no method bodies, no block
      // call sites of any kind.
      Item::Enum(_) => {}
      // Plan 54: an actor's methods are ordinary method bodies, exactly
      // like a class's own arm above.
      Item::Actor(a) => {
        for m in &a.methods {
          scan_block_call_sites(&m.body, sigs, classes, func_defs, gctx, &mut diags);
        }
      }
      Item::Error => {}
      // Plan 59: an extern declaration has no body of its own — no
      // block-attaching call sites to scan.
      Item::Extern(_) => {}
    }
  }
  diags
}

fn scan_block_call_sites(
  stmts: &[Spanned<Stmt>],
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  for s in stmts {
    scan_block_call_site(s, sigs, classes, func_defs, gctx, diags);
  }
}

fn scan_block_call_site(
  stmt: &Spanned<Stmt>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  match &stmt.node {
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) => {
      let declares_block = sigs.get(name).is_some_and(|s| s.block_param.is_some());
      if declares_block {
        check_one_block_call_site(name, args, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      scan_block_call_sites(then_branch, sigs, classes, func_defs, gctx, diags);
      if let Some(else_b) = else_branch {
        scan_block_call_sites(else_b, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::While { body, .. } => scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags);
      for rescue in rescues {
        scan_block_call_sites(&rescue.body, sigs, classes, func_defs, gctx, diags);
      }
      if let Some(ensure_body) = ensure {
        scan_block_call_sites(ensure_body, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags);
      }
      if let Some(else_b) = else_body {
        scan_block_call_sites(else_b, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::For { body, .. } => scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags),
    _ => {}
  }
}

fn check_one_block_call_site(
  name: &str,
  args: &[Spanned<Expr>],
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  let Some(Spanned {
    node: Expr::Lambda {
      params: blk_params,
      body: blk_body,
      ..
    },
    ..
  }) = args.last()
  else {
    let span = args.last().map(|a| a.span).unwrap_or((0, 0));
    diags.push(Diagnostic::new(
      format!(
        "`{name}` requires a trailing block (`do |params| ... end`) — it declares a block parameter"
      ),
      span,
    ));
    return;
  };
  let mut blk_env: HashMap<String, Type> = HashMap::new();
  let mut blk_param_types = Vec::with_capacity(blk_params.len());
  for p in blk_params {
    let t = match resolve_type(&p.ty, classes) {
      Ok(t) => t,
      Err(d) => {
        diags.push(d);
        return;
      }
    };
    blk_param_types.push(t.clone());
    blk_env.insert(p.name.clone(), t);
  }
  // Plan 72's Decision log: a block literal's own parameter is
  // immutable by construction, same as a lambda's — fresh, empty set.
  let mut blk_mutable: HashSet<String> = HashSet::new();
  if let Err(d) = check_block(
    blk_body,
    &mut blk_env,
    &mut blk_mutable,
    sigs,
    classes,
    None,
    &Type::Void,
    false,
    false,
    false,
    gctx,
  ) {
    diags.push(d);
    return;
  }
  let Some(callee) = func_defs.get(name) else {
    return;
  };
  let mut callee_env: HashMap<String, Type> = HashMap::new();
  for p in &callee.params {
    let t = match resolve_type(&p.ty, classes) {
      Ok(t) => t,
      Err(d) => {
        diags.push(d);
        return;
      }
    };
    callee_env.insert(p.name.clone(), t);
  }
  if let Err(d) = check_yields_against_block(
    &callee.body,
    &blk_param_types,
    &mut callee_env,
    sigs,
    classes,
    gctx,
  ) {
    diags.push(d);
  }
}

/// Re-walks a `block_param`-declaring function's body, tracking the
/// same flat local-variable environment `check_block` would (so a
/// `yield` site referencing an earlier `Let`-bound local still
/// type-checks), but checking only `Stmt::Yield` sites — every other
/// statement kind was already fully checked once by this function's own
/// routine `check_function_body` pass; re-verifying it here would be
/// redundant, not incorrect.
fn check_yields_against_block(
  stmts: &[Spanned<Stmt>],
  expected: &[Type],
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    match &stmt.node {
      Stmt::Yield(args) => {
        if args.len() != expected.len() {
          let span = args.first().map(|a| a.span).unwrap_or(stmt.span);
          return Err(Diagnostic::new(
            format!(
              "block arity mismatch: `yield` passes {} argument(s), attached block declares {} parameter(s)",
              args.len(),
              expected.len()
            ),
            span,
          ));
        }
        for (i, (a, want)) in args.iter().zip(expected).enumerate() {
          let actual = infer_expr_type(a, env, sigs, classes, None, gctx)?;
          if actual != *want {
            return Err(Diagnostic::new(
              format!(
                "type mismatch in `yield` argument {}: attached block's parameter has type {want:?}, value has type {actual:?}",
                i + 1
              ),
              a.span,
            ));
          }
        }
      }
      Stmt::Let { name, ty, .. } => {
        if let Ok(t) = resolve_type(ty, classes) {
          env.insert(name.clone(), t);
        }
      }
      Stmt::If {
        then_branch,
        else_branch,
        ..
      } => {
        check_yields_against_block(then_branch, expected, env, sigs, classes, gctx)?;
        if let Some(else_b) = else_branch {
          check_yields_against_block(else_b, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::While { body, .. } => {
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?
      }
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
        for rescue in rescues {
          if let Some(class_name) = &rescue.class_name {
            if let Ok(t) = resolve_type(&TypeExpr::Named(class_name.clone()), classes) {
              env.insert(rescue.var.clone(), t);
            }
          }
          check_yields_against_block(&rescue.body, expected, env, sigs, classes, gctx)?;
        }
        if let Some(ensure_body) = ensure {
          check_yields_against_block(ensure_body, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::Case {
        arms, else_body, ..
      } => {
        for (_, body) in arms {
          check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
        }
        if let Some(else_b) = else_body {
          check_yields_against_block(else_b, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::For {
        var,
        elements,
        body,
      } => {
        if let Ok(Type::Array(elem_ty)) =
          infer_array_lit_type(elements, env, sigs, classes, None, gctx)
        {
          env.insert(var.clone(), *elem_ty);
        }
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
      }
      _ => {}
    }
  }
  Ok(())
}

/// Type-checks a full program: registers classes (two sub-passes — names
/// first so field/method types can reference class names, then full
/// field/method tables) and free-function signatures, then checks every
/// function body, every class's method bodies, and every top-level
/// statement. `puts` is handled directly in `infer_expr_type`, not
/// registered here (see plan 08's Decision log) — free functions are
/// enough two-pass ordering for a top-level call to a function defined
/// later in the same `Item*` list to still resolve.
/// One function or method's public signature (plan 21's Decision log:
/// a thin DTO over the private `FunctionSig` this crate already
/// computes internally, not new analysis).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSymbol {
  pub params: Vec<Type>,
  pub return_type: Type,
}

/// One class or module's public shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassSymbol {
  pub is_module: bool,
  pub fields: HashMap<String, Type>,
  pub methods: HashMap<String, FunctionSymbol>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolTable {
  pub functions: HashMap<String, FunctionSymbol>,
  pub classes: HashMap<String, ClassSymbol>,
}

fn function_symbol_from_sig(sig: FunctionSig) -> FunctionSymbol {
  FunctionSymbol {
    params: sig.params,
    return_type: sig.return_type,
  }
}

/// Plan 21's Decision log: a thin, disclosed export of the exact same
/// two-pass class/module + non-generic-function registration
/// `check_program` already runs internally before it ever checks a
/// single body — best-effort and never fails on a body-level type
/// error (this never checks a body at all, only signatures), and a
/// per-declaration graceful degrade: a class/function whose own
/// signature fails to resolve is simply omitted from the returned
/// table, not an all-or-nothing abort. Implemented as its own
/// self-contained duplicate of `check_program`'s registration passes,
/// deliberately not a literal shared-helper refactor of `check_program`
/// itself — zero risk of changing that function's own, already-tested
/// behavior to get a read-only export of what it already computes.
/// Interfaces/generic functions are out of scope here — `SymbolTable`
/// has no shape for either, matching how `check_program`'s own
/// non-generic `sigs` map already excludes a generic function's name.
pub fn collect_symbols(program: &Program) -> SymbolTable {
  let mut classes: HashMap<String, ClassInfo> = HashMap::new();
  let mut class_defs: HashMap<String, &ClassDef> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      class_defs.insert(c.name.clone(), c);
      classes.insert(
        c.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: c.superclass.clone(),
          implements: c.implements.clone(),
          enum_variants: None,
          is_actor: false,
          generic_methods: HashMap::new(),
        },
      );
    }
    if let Item::Module(m) = item {
      classes.insert(
        m.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: true,
          superclass: None,
          implements: None,
          enum_variants: None,
          is_actor: false,
          generic_methods: HashMap::new(),
        },
      );
    }
    if let Item::Actor(a) = item {
      classes.insert(
        a.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: None,
          implements: None,
          enum_variants: None,
          is_actor: true,
          generic_methods: HashMap::new(),
        },
      );
    }
  }

  let mut resolved_classes: HashMap<String, ClassInfo> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      if let Ok(info) = build_flattened_class_info(&c.name, &class_defs, &classes, &HashMap::new())
      {
        classes.insert(c.name.clone(), info.clone());
        resolved_classes.insert(c.name.clone(), info);
      }
    }
    if let Item::Module(m) = item {
      if let Ok(info) = module_info(m, &classes) {
        classes.insert(m.name.clone(), info.clone());
        resolved_classes.insert(m.name.clone(), info);
      }
    }
    if let Item::Actor(a) = item {
      if let Ok(info) = actor_info(a, &classes) {
        classes.insert(a.name.clone(), info.clone());
        resolved_classes.insert(a.name.clone(), info);
      }
    }
  }

  let mut functions: HashMap<String, FunctionSymbol> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      if f.type_params.is_empty() {
        if let Ok(sig) = function_signature(f, &classes) {
          functions.insert(f.name.clone(), function_symbol_from_sig(sig));
        }
      }
    }
  }

  let classes = resolved_classes
    .into_iter()
    .map(|(name, info)| {
      let methods = info
        .methods
        .into_iter()
        .map(|(m_name, sig)| (m_name, function_symbol_from_sig(sig)))
        .collect();
      (
        name,
        ClassSymbol {
          is_module: info.is_module,
          fields: info.fields,
          methods,
        },
      )
    })
    .collect();

  SymbolTable { functions, classes }
}

pub fn check_program(program: &Program) -> Result<(), Vec<Diagnostic>> {
  let mut diags = Vec::new();

  // Plan 41: interfaces register in their own pass, independent of
  // class/module registration order — an interface's required method
  // is kept as raw, unresolved type-name strings (`InterfaceInfo`'s
  // own doc comment), so nothing here needs `classes` populated yet.
  let mut interfaces: HashMap<String, InterfaceInfo> = HashMap::new();
  for item in &program.items {
    if let Item::Interface(idef) = item {
      interfaces.insert(
        idef.name.clone(),
        InterfaceInfo {
          type_params: idef.type_params.clone(),
          methods: idef
            .methods
            .iter()
            .map(|m| InterfaceMethodInfo {
              method_name: m.method_name.clone(),
              type_params: m.type_params.clone(),
              params_raw: m
                .params
                .iter()
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect(),
              return_type_raw: m.return_type.clone(),
            })
            .collect(),
        },
      );
    }
  }

  // Modules register into the same two-pass table as classes (plan 12's
  // Decision log) — names first (so a class/module's own fields/methods
  // can reference any other class/module name regardless of declaration
  // order), then full field/method tables.
  let mut classes: HashMap<String, ClassInfo> = HashMap::new();
  // Plan 32: raw `ClassDef`s keyed by name, so `build_flattened_class_info`
  // can look up any ancestor's own field/method declarations directly —
  // independent of `program.items`' order, unlike a scheme that only
  // ever has each *previously computed* `ClassInfo` to work from.
  let mut class_defs: HashMap<String, &ClassDef> = HashMap::new();
  // Plan 58: a class with a non-empty `type_params` is a generic
  // TEMPLATE — routed into this separate registry instead of `classes`/
  // `class_defs`, never monomorphized without a real, concrete
  // instantiation actually written somewhere in the program (this
  // struct's own doc comment).
  let mut generic_classes: HashMap<String, &ClassDef> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      if !c.type_params.is_empty() {
        generic_classes.insert(c.name.clone(), c);
        continue;
      }
      class_defs.insert(c.name.clone(), c);
      classes.insert(
        c.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: c.superclass.clone(),
          implements: c.implements.clone(),
          enum_variants: None,
          is_actor: false,
          generic_methods: HashMap::new(),
        },
      );
    }
    if let Item::Module(m) = item {
      classes.insert(
        m.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: true,
          superclass: None,
          implements: None,
          enum_variants: None,
          is_actor: false,
          generic_methods: HashMap::new(),
        },
      );
    }
    if let Item::Actor(a) = item {
      classes.insert(
        a.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: None,
          implements: None,
          enum_variants: None,
          is_actor: true,
          generic_methods: HashMap::new(),
        },
      );
    }
  }
  // Plan 52: enums register into the SAME `classes` table (Decision
  // log's disclosed adaptation) — a first pass inserts every enum's
  // name with a placeholder (empty) variant list, so a variant field
  // naming another enum resolves regardless of declaration order, then
  // a second pass resolves each variant's real field types now that
  // every class/module/enum NAME is known. Rejected at registration
  // time, before any case/construction is checked: a name colliding
  // with an already-registered class/module/enum, and a variant name
  // colliding with a variant already seen in ANY enum registered so
  // far (a flat, single namespace, the same discipline `resolve_type`
  // already enforces between class and module names).
  // Plan 73: `Option[T]` — this compiler's first GENERIC enum,
  // synthesized directly in Rust (never parsed from source, so
  // `EnumVariant`'s own grammar-level "at least one field" restriction
  // doesn't apply to `None`'s zero fields) and registered into
  // `generic_enums` exactly like any user-written `enum Name[T] = ...`
  // would be — no special-casing anywhere downstream of this point.
  let option_enum_def = EnumDef {
    name: "Option".to_string(),
    variants: vec![
      EnumVariant {
        name: "Some".to_string(),
        fields: vec![TypeExpr::Named("T".to_string())],
      },
      EnumVariant {
        name: "None".to_string(),
        fields: vec![],
      },
    ],
    type_params: vec![TypeParam {
      name: "T".to_string(),
      bounds: Vec::new(),
    }],
  };
  // Plan 73: enums, like classes above, split into a generic-TEMPLATE
  // registry (never monomorphized without a real instantiation actually
  // written somewhere in the program) and the ordinary per-name
  // registration pass below.
  let mut generic_enums: HashMap<String, &EnumDef> = HashMap::new();
  generic_enums.insert("Option".to_string(), &option_enum_def);
  let mut enum_defs: Vec<&EnumDef> = Vec::new();
  let mut seen_variant_names: HashSet<String> = HashSet::new();
  for item in &program.items {
    if let Item::Enum(e) = item {
      if !e.type_params.is_empty() {
        if generic_enums.contains_key(&e.name) {
          diags.push(Diagnostic::new(
            format!("`{}` is already declared as a generic enum", e.name),
            (0, 0),
          ));
          continue;
        }
        generic_enums.insert(e.name.clone(), e);
        continue;
      }
      if classes.contains_key(&e.name) {
        diags.push(Diagnostic::new(
          format!(
            "`{}` is already declared as a class, module, or enum",
            e.name
          ),
          (0, 0),
        ));
        continue;
      }
      let mut ok = true;
      for v in &e.variants {
        if !seen_variant_names.insert(v.name.clone()) {
          diags.push(Diagnostic::new(
            format!(
              "variant `{}` of enum `{}` collides with a variant already declared elsewhere",
              v.name, e.name
            ),
            (0, 0),
          ));
          ok = false;
        }
      }
      if !ok {
        continue;
      }
      classes.insert(
        e.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: None,
          implements: None,
          enum_variants: Some(Vec::new()),
          is_actor: false,
          generic_methods: HashMap::new(),
        },
      );
      enum_defs.push(e);
    }
  }
  for e in &enum_defs {
    let mut resolved_variants = Vec::new();
    let mut ok = true;
    for v in &e.variants {
      let mut field_types = Vec::new();
      for f in &v.fields {
        match resolve_type(f, &classes) {
          Ok(t) => field_types.push(t),
          Err(d) => {
            diags.push(d);
            ok = false;
          }
        }
      }
      resolved_variants.push((v.name.clone(), field_types));
    }
    if ok {
      if let Some(info) = classes.get_mut(&e.name) {
        info.enum_variants = Some(resolved_variants);
      }
    }
  }
  for item in &program.items {
    // Plan 58: a generic class's own `ClassInfo` is never flattened
    // here — it doesn't exist in `classes`/`class_defs` at all (routed
    // into `generic_classes` above instead), monomorphized on demand by
    // the collect+instantiate pass just below.
    if let Item::Class(c) = item {
      if c.type_params.is_empty() {
        match build_flattened_class_info(&c.name, &class_defs, &classes, &interfaces) {
          Ok(info) => {
            classes.insert(c.name.clone(), info);
          }
          Err(d) => diags.push(d),
        }
      }
    }
    if let Item::Module(m) = item {
      match module_info(m, &classes) {
        Ok(info) => {
          classes.insert(m.name.clone(), info);
        }
        Err(d) => diags.push(d),
      }
    }
    if let Item::Actor(a) = item {
      match actor_info(a, &classes) {
        Ok(info) => {
          classes.insert(a.name.clone(), info);
        }
        Err(d) => diags.push(d),
      }
    }
  }

  // Plan 41: checked immediately after a class with `implements.is_some()`
  // successfully registers its flattened `ClassInfo` (Decision log) — a
  // class whose OWN registration failed (`classes.get` came back `None`
  // above) never reaches this at all, matching the established "if
  // registration failed, skip downstream checks" pattern.
  for item in &program.items {
    if let Item::Class(c) = item {
      if c.implements.is_some() {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        if let Err(d) = check_interface_conformance(&c.name, info, &interfaces, &classes) {
          diags.push(d);
        }
      }
    }
  }

  // Plan 58: resolves every generic-class instantiation actually
  // written anywhere in the program (`Let` annotations, ordinary
  // function/method/actor param/return types, class/actor field types)
  // into a real, monomorphized `ClassInfo`, inserted into `classes`
  // under its mangled name — BEFORE `sigs`/`generic_sigs` register
  // just below (a function signature may itself reference one, e.g.
  // `def make() -> Stack[Int64]`).
  for ty in collect_generic_instantiation_typenames(program) {
    if let TypeExpr::Generic(base, args) = &ty {
      if generic_classes.contains_key(base.as_str()) {
        let mut in_progress = Vec::new();
        if let Err(d) =
          instantiate_generic_class(base, args, &generic_classes, &mut classes, &mut in_progress)
        {
          diags.push(d);
        }
      } else if generic_enums.contains_key(base.as_str()) {
        // Plan 73: the identical discovery mechanism above, extended to
        // a generic ENUM — `Option[Int64]` written anywhere as a `Let`/
        // param/return/field type annotation triggers its own
        // monomorphization here, exactly the same way `Stack[Int64]`
        // already does for a generic class.
        let mut in_progress = Vec::new();
        if let Err(d) = instantiate_generic_enum(
          base,
          args,
          &generic_classes,
          &generic_enums,
          &mut classes,
          &mut in_progress,
        ) {
          diags.push(d);
        }
      }
    }
  }
  // Plan 73: `String.from_cstring`'s own real return type is `Option[
  // String]` (see that call site's Decision log) — unconditionally
  // pre-instantiated here, regardless of whether this specific program
  // ever writes `"Option[String]"` as literal annotation text anywhere,
  // since that's the one case this compiler derives an `Option[T]`
  // internally rather than from a textual type annotation the discovery
  // pass above can see.
  {
    let mut in_progress = Vec::new();
    if let Err(d) = instantiate_generic_enum(
      "Option",
      &[TypeExpr::Named("String".to_string())],
      &generic_classes,
      &generic_enums,
      &mut classes,
      &mut in_progress,
    ) {
      diags.push(d);
    }
  }

  let mut sigs: HashMap<String, FunctionSig> = HashMap::new();
  // Plan 41: a top-level function whose `type_params` is non-empty is a
  // generic function — it never enters `sigs` at all (Decision log: the
  // ordinary `sigs` pass skips it entirely, so its name is never present
  // in both registries at once). Exactly one type parameter is required;
  // two or more is a real registration-time diagnostic, and the offending
  // function's name is tracked here so the later body-check loop skips
  // it too (already diagnosed once, not silently, not twice).
  let mut generic_sigs: HashMap<String, GenericFunctionSig> = HashMap::new();
  let mut bad_generic_fns: HashSet<String> = HashSet::new();
  // Plan 34: raw `Function`s keyed by name, alongside `sigs` — a
  // block-attaching call site needs the callee's actual body (to
  // re-walk its `Stmt::Yield` sites), not just its signature.
  let mut func_defs: HashMap<String, &Function> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      func_defs.insert(f.name.clone(), f);
      if f.type_params.is_empty() {
        match function_signature(f, &classes) {
          Ok(sig) => {
            sigs.insert(f.name.clone(), sig);
          }
          Err(d) => diags.push(d),
        }
      } else if f.type_params.len() != 1 {
        diags.push(Diagnostic::new(
          format!(
            "generic function `{}` declares {} type parameters — multiple type parameters are not supported",
            f.name,
            f.type_params.len()
          ),
          (0, 0),
        ));
        bad_generic_fns.insert(f.name.clone());
      } else {
        let tp = &f.type_params[0];
        // Plan 58's Decision log: `TypeParam.bounds` widened to a real
        // set for generic CLASSES' own bound-less case (`class Box[T]`)
        // and plan 88's multi-bound case, but a top-level generic
        // FUNCTION still requires AT LEAST ONE bound — a bound-less
        // type parameter here is rejected by this sema-level diagnostic
        // instead of structurally by the grammar (the same "grammar
        // admits a superset, sema narrows" discipline this codebase
        // already uses elsewhere).
        if tp.bounds.is_empty() {
          diags.push(Diagnostic::new(
            format!(
              "generic function `{}` declares type parameter `{}` with no bound — generic functions require a bound (e.g. `[T: Comparable]`); an unbounded type parameter is only supported on a generic CLASS",
              f.name, tp.name
            ),
            (0, 0),
          ));
          bad_generic_fns.insert(f.name.clone());
          continue;
        };
        generic_sigs.insert(
          f.name.clone(),
          GenericFunctionSig {
            type_param: tp.name.clone(),
            bounds: tp.bounds.clone(),
            params_raw: f
              .params
              .iter()
              .map(|p| (p.name.clone(), p.ty.clone()))
              .collect(),
            return_type_raw: f.return_type.clone(),
          },
        );
      }
    }
  }

  // Plan 59: an `unsafe extern "C" { ... }` fn registers into this SAME
  // `sigs` registry `function_signature()` populates for ordinary `def`
  // functions (Decision log) — no new call-expression AST is needed;
  // `llabs(x)`-style calls resolve through the existing `Expr::Call`
  // path unchanged. Also checked here: the block's own ABI literal
  // (only exactly `"C"` is supported — the explicit C++ decline).
  for item in &program.items {
    let Item::Extern(block) = item else { continue };
    if block.abi != "C" {
      diags.push(Diagnostic::new(
        format!(
          "unsupported extern ABI `\"{}\"` — only `\"C\"` is supported",
          block.abi
        ),
        (0, 0),
      ));
      continue;
    }
    for f in &block.fns {
      let params: Result<Vec<Type>, Diagnostic> = f
        .params
        .iter()
        .map(|p| resolve_extern_type(&p.ty, false))
        .collect();
      let params = match params {
        Ok(p) => p,
        Err(d) => {
          diags.push(d);
          continue;
        }
      };
      let return_type = match resolve_extern_type(&f.return_type, true) {
        Ok(t) => t,
        Err(d) => {
          diags.push(d);
          continue;
        }
      };
      sigs.insert(
        f.name.clone(),
        FunctionSig {
          params,
          return_type,
          block_param: None,
          param_names: f.params.iter().map(|p| p.name.clone()).collect(),
          defaults: vec![None; f.params.len()],
          splat_elem: None,
          requires: Vec::new(),
          // Plan 63's Decision log, forbidden category 3: an extern
          // "C" FFI call is never provably pure — no `pure` keyword
          // is even grammatically reachable on an `ExternFn`, so this
          // is always `false`, never read from user source.
          is_pure: false,
        },
      );
    }
  }

  let gctx = GenericsCtx {
    interfaces: &interfaces,
    generic_sigs: &generic_sigs,
  };

  // Plan 58: a generic class TEMPLATE's method bodies check exactly
  // once each, against `Type::Generic`-substituted field/param/return
  // types (mirroring the generic-FUNCTION template check just below) —
  // never once per instantiation. Needs `sigs`/`gctx` already built
  // (a template method can call an ordinary function or another
  // generic function), so this runs here, not alongside the collect+
  // instantiate pass above.
  for c in generic_classes.values() {
    if let Err(d) = check_generic_class_body(c, &sigs, &classes, &gctx) {
      diags.push(d);
    }
  }

  // Plan 52: the other half of the registration-time collision check —
  // a variant name colliding with a declared function is only
  // detectable now that `sigs`/`generic_sigs` are both built. Enum-vs-
  // class/module and variant-vs-variant collisions were already
  // rejected earlier, before this point, so `classes` here reflects
  // only successfully-registered enums.
  for info in classes.values() {
    let Some(variants) = &info.enum_variants else {
      continue;
    };
    for (variant_name, _) in variants {
      if sigs.contains_key(variant_name) || generic_sigs.contains_key(variant_name) {
        diags.push(Diagnostic::new(
          format!("variant `{variant_name}` collides with a declared function of the same name"),
          (0, 0),
        ));
      }
    }
  }

  // Declared once, outside the loop: top-level statements share one
  // environment across the whole program in order (`x: Int64 = 10` then
  // `if x > 5 ...` needs `x` visible in a later Item::Stmt).
  // Plan 45's Decision log: `ARGV`/`ARGC` need no new AST-visiting code
  // at all — pre-seeding `top_env` is enough for the existing `Expr::
  // Ident` arm's ordinary `env.get(name)` lookup to resolve both with
  // zero special-casing beyond this seed.
  let mut top_env: HashMap<String, Type> = HashMap::new();
  top_env.insert("ARGV".to_string(), Type::Array(Box::new(Type::String)));
  top_env.insert("ARGC".to_string(), Type::Int64);
  // Plan 72's Decision log: declared once, alongside `top_env` itself,
  // so a top-level `var`-declared binding stays reassignable across
  // later `Item::Stmt`s the same way `top_env`'s own types already do.
  let mut top_mutable: HashSet<String> = HashSet::new();
  // Plan 61's Decision log: every top-level function's own name known to
  // be `is_comptime: true` — `check_comptime_legal`'s own `Call` arm
  // uses this to make comptime evaluation compositional (a `comptime`
  // function may call another `comptime` function, never an ordinary
  // one).
  let comptime_fns: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Function(f) if f.is_comptime => Some(f.name.clone()),
      _ => None,
    })
    .collect();
  for item in &program.items {
    match item {
      Item::Function(f) if !f.type_params.is_empty() => {
        if bad_generic_fns.contains(&f.name) {
          continue;
        }
        let g = &generic_sigs[&f.name];
        if let Err(d) = check_generic_function_body(f, g, &sigs, &classes, &gctx) {
          diags.push(d);
        }
      }
      Item::Function(f) => {
        if f.is_comptime {
          if let Err(d) = check_comptime_legal(&f.body, &comptime_fns) {
            diags.push(d);
          }
        }
        if let Err(d) = check_function_body(f, &sigs, &classes, &gctx) {
          diags.push(d);
        }
      }
      Item::Class(c) => {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        for m in &c.methods {
          // Plan 88's Decision log: a generic method (routed into
          // `info.generic_methods`, never `info.methods`) is skipped
          // here — the same real, disclosed body-checking gap
          // `check_generic_class_body` already documents for a generic
          // CLASS template's own generic methods, extended to an
          // ordinary (non-generic) class's generic method too. Only
          // call-site argument/return-type checking is implemented
          // (`infer_expr_type`'s `generic_methods` dispatch).
          if !m.type_params.is_empty() {
            continue;
          }
          if let Err(d) = check_method_body(&c.name, m, &sigs, &classes, &info.fields, &gctx) {
            diags.push(d);
          }
        }
      }
      // A module method type-checks exactly like a free function (plan
      // 12's Decision log) — no `self`, no `@field` access.
      Item::Module(m) => {
        for f in &m.methods {
          if let Err(d) = check_function_body(f, &sigs, &classes, &gctx) {
            diags.push(d);
          }
        }
      }
      // Plan 54: an actor's method bodies check exactly like a class's
      // own methods — real `self`/`@field` access, same `check_method_body`.
      Item::Actor(a) => {
        let Some(info) = classes.get(&a.name) else {
          continue;
        };
        for m in &a.methods {
          if let Err(d) = check_method_body(&a.name, m, &sigs, &classes, &info.fields, &gctx) {
            diags.push(d);
          }
        }
      }
      // No enclosing function return type (Void is a safe sentinel — a
      // bare `return` at top level is not exercised by this milestone),
      // and never inside a method (`self_fields: None`).
      Item::Stmt(s) => {
        if let Err(d) = check_stmt(
          s,
          &mut top_env,
          &mut top_mutable,
          &sigs,
          &classes,
          None,
          &Type::Void,
          false,
          false,
          false,
          &gctx,
        ) {
          diags.push(d);
        }
      }
      // Plan 41: a general, user-declarable grammar production (Decision
      // log) — nothing left to do here beyond the two registration
      // passes above; an interface has no body of its own to check.
      Item::Interface(_) => {}
      // Plan 23: see `check_block_call_sites`'s own `Item::Require`
      // arm — a no-op here too, for the same reason.
      Item::Require(_) => {}
      // Plan 47's Decision log: a `test` body type-checks as a fresh,
      // `Void`-return, no-`self`-fields scope — the exact same shape
      // `check_function_body` already gives a free function, so this
      // just synthesizes one rather than duplicating that logic.
      Item::Test { body, .. } => {
        let synthetic = Function {
          name: "test".to_string(),
          params: Vec::new(),
          return_type: TypeExpr::Named("Void".to_string()),
          body: body.clone(),
          block_param: None,
          splat_param: None,
          type_params: Vec::new(),
          is_comptime: false,
          requires: Vec::new(),
          ensures: Vec::new(),
          is_pure: false,
        };
        if let Err(d) = check_function_body(&synthetic, &sigs, &classes, &gctx) {
          diags.push(d);
        }
      }
      // Plan 52: an enum has no body of its own to check beyond the
      // registration-time checks already performed above.
      Item::Enum(_) => {}
      // Plan 26's Decision log: `emerald_parser::parse`/`parse_named`
      // returns `Ok(program)` only when zero errors were recovered —
      // `program.items` then contains no `Item::Error` by construction,
      // so `check_program` never actually receives one.
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
      // Plan 59: registered into `sigs` above, already validated
      // (ABI, param/return type allow-list) at registration time — no
      // body of its own to check.
      Item::Extern(_) => {}
    }
  }

  diags.extend(check_block_call_sites(
    program, &sigs, &classes, &func_defs, &gctx,
  ));

  diags.extend(check_comptime_positions(program));

  // Plan 63's `leaf-sema-purity-check` — runs last, after every
  // `check_function_body`/`check_method_body` call above has already
  // used `sigs`/`classes` to type-check the whole program (Decision
  // log: mirrors exactly when plan 56's `check_message_safety` already
  // runs relative to ordinary type-checking).
  if let Err(purity_diags) = check_purity(program, &sigs, &classes, &gctx) {
    diags.extend(purity_diags);
  }

  if diags.is_empty() {
    Ok(())
  } else {
    Err(diags)
  }
}

/// Plan 41's Decision log: substitutes every `"Self"` in the interface's
/// raw parameter/return-type strings with `class_name` itself, resolves
/// the substituted strings, and compares the result *exactly* (invariant,
/// not covariant — same discipline as plan 32's override check) against
/// `info.methods.get(...)` — the class's already-**flattened** method
/// table, so an interface requirement satisfied by an *inherited* method
/// is accepted for free.
///
/// Plan 89's Decision log widens this from a single required method to
/// the interface's FULL method set, and adds a second substitution
/// alongside `Self`: every one of the interface's own `type_params`
/// (`interface Iterable[T]`'s `T`) is bound to whatever concrete type
/// the class's own `implements Iterable[Int64]` clause supplies, before
/// either kind of method (ordinary or the method's own further-generic
/// `[U]`) is checked.
///
/// A required method that itself declares a type parameter (`fn map[U]
/// (f: Proc[T, U]): Array[U]`) is checked structurally against the
/// class's own `generic_methods` registry (already built with the same
/// `T` substitution applied — `build_flattened_class_info`'s own
/// `interface_type_param_subst` call) rather than resolved into a real
/// `Type` — a real, disclosed simplification: the class's own
/// implementing method must spell its type parameter identically to the
/// interface's (`U` here), since no renaming/unification between two
/// differently-named method type parameters is attempted.
fn check_interface_conformance(
  class_name: &str,
  info: &ClassInfo,
  interfaces: &HashMap<String, InterfaceInfo>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<(), Diagnostic> {
  let (iface_name, iface_args) = info
    .implements
    .as_ref()
    .expect("caller only invokes this for a class with implements.is_some()");
  let iface = interfaces.get(iface_name.as_str()).ok_or_else(|| {
    Diagnostic::new(
      format!("class `{class_name}` declares `implements {iface_name}`, but no interface named `{iface_name}` is declared"),
      (0, 0),
    )
  })?;
  if iface_args.len() != iface.type_params.len() {
    return Err(Diagnostic::new(
      format!(
        "class `{class_name}` declares `implements {iface_name}` with {} type argument(s), but interface `{iface_name}` declares {} type parameter(s)",
        iface_args.len(),
        iface.type_params.len()
      ),
      (0, 0),
    ));
  }
  // Plan 88: `Self` substitution is a real, recursive `TypeExpr` tree
  // rewrite (`substitute_type_params`) rather than a whole-string
  // equality check — a compound interface method signature referencing
  // `Self` nested inside another shape (`Array[Self]`, say) substitutes
  // correctly too. Plan 89 adds the interface's own type parameter(s)
  // to this same substitution map.
  let self_texpr = TypeExpr::Named(class_name.to_string());
  let mut subst: HashMap<&str, &TypeExpr> = HashMap::new();
  subst.insert("Self", &self_texpr);
  for (tp, arg) in iface.type_params.iter().zip(iface_args.iter()) {
    subst.insert(tp.name.as_str(), arg);
  }
  for m in &iface.methods {
    let expected_params_raw: Vec<(String, TypeExpr)> = m
      .params_raw
      .iter()
      .map(|(n, raw)| (n.clone(), substitute_type_params(raw, &subst)))
      .collect();
    let expected_return_raw = substitute_type_params(&m.return_type_raw, &subst);
    if m.type_params.is_empty() {
      let expected_params = expected_params_raw
        .iter()
        .map(|(_, raw)| resolve_type(raw, classes))
        .collect::<Result<Vec<_>, _>>()?;
      let expected_return = resolve_type(&expected_return_raw, classes)?;
      let Some(actual) = info.methods.get(&m.method_name) else {
        return Err(Diagnostic::new(
          format!(
            "class `{class_name}` declares `implements {iface_name}` but does not define required method `{}`",
            m.method_name
          ),
          (0, 0),
        ));
      };
      if actual.params != expected_params || actual.return_type != expected_return {
        return Err(Diagnostic::new(
          format!(
            "class `{class_name}`'s `{}` does not match interface `{iface_name}`'s required signature: expected {expected_params:?} -> {expected_return:?}, found {:?} -> {:?}",
            m.method_name, actual.params, actual.return_type
          ),
          (0, 0),
        ));
      }
    } else {
      let Some(actual) = info.generic_methods.get(&m.method_name) else {
        return Err(Diagnostic::new(
          format!(
            "class `{class_name}` declares `implements {iface_name}` but does not define required generic method `{}`",
            m.method_name
          ),
          (0, 0),
        ));
      };
      if m.type_params.len() != 1 {
        return Err(Diagnostic::new(
          format!(
            "interface `{iface_name}`'s generic method `{}` declares more than one type parameter — not supported",
            m.method_name
          ),
          (0, 0),
        ));
      }
      let expected_params: Vec<TypeExpr> =
        expected_params_raw.iter().map(|(_, t)| t.clone()).collect();
      let actual_params: Vec<TypeExpr> = actual.params_raw.iter().map(|(_, t)| t.clone()).collect();
      if actual.type_param != m.type_params[0].name
        || expected_params != actual_params
        || expected_return_raw != actual.return_type_raw
      {
        return Err(Diagnostic::new(
          format!(
            "class `{class_name}`'s generic method `{}` does not match interface `{iface_name}`'s required signature",
            m.method_name
          ),
          (0, 0),
        ));
      }
    }
  }
  Ok(())
}

/// Plan 41's Decision log: a generic function's body is type-checked
/// exactly **once**, statically, before any concrete type is known — a
/// parameter (or return type) whose declared type name equals the
/// function's own type parameter is bound to `Type::Generic(name, bound)`
/// directly (bypassing `resolve_type`, which cannot resolve a bare
/// `"T"`), so a method call on it resolves against the bound interface's
/// required signature (`infer_expr_type`'s `Expr::MethodCall` arm), not
/// against the `classes` registry.
fn check_generic_function_body(
  f: &Function,
  g: &GenericFunctionSig,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  // Plan 62's Decision log: a `type_params`-bearing function is compiled
  // via whole-program monomorphization and never reaches `check_
  // function_body` at all (the top-level dispatch loop routes it here
  // instead) — this is the one real place that dispatch decision can be
  // observed, so this is where the "not supported on generic functions
  // yet" diagnostic has to live, even though `requires`/`ensures`
  // themselves are otherwise entirely `check_function_body`'s concern.
  if !f.requires.is_empty() || !f.ensures.is_empty() {
    return Err(Diagnostic::new(
      format!(
        "contracts are not supported on generic functions yet (`{}`)",
        f.name
      ),
      (0, 0),
    ));
  }
  let mut env = HashMap::new();
  for p in &f.params {
    let t = if p.ty.as_named() == Some(g.type_param.as_str()) {
      Type::Generic(g.type_param.clone(), g.bounds.clone())
    } else {
      resolve_type(&p.ty, classes)?
    };
    env.insert(p.name.clone(), t);
  }
  let declared_return = if f.return_type.as_named() == Some(g.type_param.as_str()) {
    Type::Generic(g.type_param.clone(), g.bounds.clone())
  } else {
    resolve_type(&f.return_type, classes)?
  };
  // Plan 72's Decision log: a generic function's own parameter is
  // immutable by construction, same as an ordinary function's.
  let mut mutable_locals: HashSet<String> = HashSet::new();
  check_block(
    &f.body,
    &mut env,
    &mut mutable_locals,
    sigs,
    classes,
    None,
    &declared_return,
    false,
    false,
    f.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &f.body,
    &env,
    sigs,
    classes,
    None,
    &declared_return,
    &f.name,
    gctx,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_EM: &str = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn accepts_hello_em() {
    let program = emerald_parser::parse(HELLO_EM).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_inception_25e_mismatch_with_useful_diagnostic() {
    let src = "fn add(a: Int64, b: String): Int64 do\n  a + b\nend";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 + String");
    assert_eq!(errs.len(), 1);
    let msg = &errs[0].message;
    assert!(msg.contains("Int64"), "diagnostic should name Int64: {msg}");
    assert!(
      msg.contains("String"),
      "diagnostic should name String: {msg}"
    );
  }

  #[test]
  fn rejects_undefined_variable_without_panicking() {
    let src = "fn add(a: Int64): Int64 do\n  a + b\nend";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject undefined `b`");
    assert!(errs[0].message.contains("undefined variable `b`"));
  }

  #[test]
  fn rejects_arity_mismatch_at_call_site() {
    // Plan 39: a call omitting a required (no-default) trailing
    // parameter now names it directly, rather than a blanket "expects N
    // argument(s)" — a real, plan-required diagnostic improvement (AC4:
    // "a compile-time arity diagnostic naming the missing required
    // parameter").
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 1-arg call to 2-arg add");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("missing required argument `b`")));
  }

  #[test]
  fn rejects_too_many_arguments_to_a_non_splat_function() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(1, 2, 3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a 3-arg call to a 2-arg add");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("expects 2 argument")));
  }

  #[test]
  fn rejects_undefined_function_without_panicking() {
    let src = "puts undefined_fn(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject call to undefined function");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("undefined function `undefined_fn`")));
  }

  const MILESTONE2: &str = "x: Int64 = 10\n\nif x > 5 do\n  puts x\nend\n";

  #[test]
  fn accepts_inception_milestone2_example() {
    let program = emerald_parser::parse(MILESTONE2).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_boolean_if_condition() {
    let src = "x: Int64 = 10\n\nif x do\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject non-Boolean if condition");
    assert!(errs[0].message.contains("must be Boolean"));
  }

  #[test]
  fn rejects_break_outside_loop() {
    let src = "break\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject break outside a loop");
    assert!(errs[0].message.contains("outside of a loop"));
  }

  #[test]
  fn accepts_while_loop_with_break() {
    let src = "x: Int64 = 0\n\nwhile x < 3 do\n  x: Int64 = x + 1\n  break\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn sum: Float64 do\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn accepts_inception_point_example() {
    let program = emerald_parser::parse(POINT_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_field_assignment_type_mismatch() {
    let src =
      "class Point\n  x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = 1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 assigned to a Float64 field");
    let msg = &errs[0].message;
    assert!(
      msg.contains("Float64") && msg.contains("Int64"),
      "diagnostic should name both types: {msg}"
    );
  }

  #[test]
  fn rejects_instance_var_outside_method_body() {
    let src = "puts @x\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject @x outside a method body");
    assert!(errs[0].message.contains("outside of a method body"));
  }

  #[test]
  fn rejects_undeclared_method_call() {
    let src = "class Point\n  x: Float64\nend\n\np: Point = Point.new()\nputs p.missing\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a call to an undeclared method");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("has no method `missing`")));
  }

  #[test]
  fn rejects_method_call_on_non_class_receiver() {
    let src = "x: Int64 = 5\nputs x.sum\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject .sum on an Int64 receiver");
    assert!(errs.iter().any(|d| d.message.contains("non-class type")));
  }

  const ARRAY_EXAMPLE: &str =
    "arr: Array[Int64] = [10, 20, 30]\narr[1] = 99\nputs arr[1]\nputs arr[0] + arr[2]\n";

  #[test]
  fn accepts_array_literal_index_read_and_write() {
    let program = emerald_parser::parse(ARRAY_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_empty_array_literal() {
    let src = "arr: Array[Int64] = []\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty array literal");
    assert!(errs[0].message.contains("empty array literals"));
  }

  #[test]
  fn rejects_mixed_type_array_literal() {
    let src = "arr: Array[Int64] = [1, 2.0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a mixed-type array literal");
    assert!(errs[0].message.contains("all elements must share one type"));
  }

  #[test]
  fn rejects_non_int64_array_index() {
    let src = "arr: Array[Int64] = [1, 2]\nputs arr[2.0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a non-Int64 array index");
    assert!(errs[0].message.contains("array index must be Int64"));
  }

  #[test]
  fn rejects_array_element_type_mismatch_on_write() {
    let src = "arr: Array[Int64] = [1, 2]\narr[0] = 2.0\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject writing a Float64 into an Array[Int64]");
    assert!(errs[0]
      .message
      .contains("type mismatch in array assignment"));
  }

  #[test]
  fn rejects_indexing_a_non_array() {
    let src = "x: Int64 = 5\nputs x[0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject indexing a non-Array");
    assert!(errs[0].message.contains("requires an Array"));
  }

  const LAMBDA_EXAMPLE: &str =
    "x: Int64 = 10\nadd_x: Proc = do |y: Int64| y + x end\nputs add_x.call(5)\n";

  #[test]
  fn accepts_lambda_capture_and_call() {
    let program = emerald_parser::parse(LAMBDA_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_arity_mismatch() {
    let src = "add_x: Proc = do |y: Int64| y end\nputs add_x.call(5, 6)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 2-arg call to a 1-param Proc");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("expects 1 argument")));
  }

  #[test]
  fn rejects_call_argument_type_mismatch() {
    let src = "add_x: Proc = do |y: Int64| y end\nputs add_x.call(1.5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a Float64 argument to an Int64 param");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("Int64") && d.message.contains("Float64")));
  }

  #[test]
  fn rejects_call_on_non_proc_receiver() {
    let src = "x: Int64 = 5\nputs x.call(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject .call on a non-Proc receiver");
    assert!(errs[0].message.contains("non-Proc type"));
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn accepts_raise_and_rescue() {
    let program = emerald_parser::parse(EXCEPTION_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_raise_of_non_class_value() {
    let src = "raise 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject raising a non-class value");
    assert!(errs[0]
      .message
      .contains("`raise` requires a class instance"));
  }

  #[test]
  fn rejects_rescue_naming_a_non_class_type() {
    let src = "begin\n  puts 1\nrescue Int64 => e\n  puts e\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `rescue Int64`");
    assert!(errs[0].message.contains("must name a class"));
  }

  const MODULE_EXAMPLE: &str = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";

  #[test]
  fn accepts_module_namespaced_call() {
    let program = emerald_parser::parse(MODULE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_instantiating_a_module() {
    let src = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `.new` on a module");
    assert!(errs[0].message.contains("cannot `.new` module"));
  }

  #[test]
  fn rejects_undeclared_module_method() {
    let src = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.missing(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undeclared module method");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("has no method `missing`")));
  }

  #[test]
  fn rejects_module_call_arity_mismatch() {
    let src = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.double(1, 2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a 2-arg call to a 1-param module method");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("expects 1 argument")));
  }

  #[test]
  fn rejects_module_used_as_a_type_annotation() {
    let src = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nx: MathUtils = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a module used as a type annotation");
    assert!(errs[0].message.contains("unknown type"));
  }

  // Plan 18 (arithmetic & logical operators).

  const ARITHMETIC_EXAMPLE: &str = "fn factorial(n: Int64): Int64 do\n  if n <= 1 do\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";
  const SHORT_CIRCUIT_EXAMPLE: &str = "fn noisy(n: Int64): Boolean do\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1) do\n  puts 100\nend\nif x > -10 && noisy(3) do\n  puts 300\nend\nif x < 0 || noisy(2) do\n  puts 200\nend\nif x > 0 || noisy(4) do\n  puts 400\nend\n";

  #[test]
  fn accepts_arithmetic_example() {
    let program = emerald_parser::parse(ARITHMETIC_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_short_circuit_example() {
    let program = emerald_parser::parse(SHORT_CIRCUIT_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_mismatched_numeric_sub_types() {
    let src = "puts 5 - 2.0\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 - Float64");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("Float64"));
  }

  #[test]
  fn rejects_not_on_non_boolean_operand() {
    let src = "puts !5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `!5`");
    assert!(errs[0].message.contains("Boolean"));
  }

  #[test]
  fn rejects_and_with_non_boolean_left_operand() {
    // No parenthesized-grouping production exists in this grammar (a
    // real, disclosed gap outside plan 18's scope) — `5 && 3 > 1`
    // still exercises the intended shape since `&&` binds looser than
    // comparison, so the right operand is `3 > 1` either way.
    let src = "if 5 && 3 > 1 do\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `5 && ...`");
    assert!(errs[0].message.contains("Boolean"));
  }

  #[test]
  fn sub_and_neg_agree_on_int64() {
    let src = "x: Int64 = 0 - 5\ny: Int64 = -5\nputs x\nputs y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn sub_and_neg_agree_on_float64() {
    let src = "x: Float64 = 0.0 - 5.0\ny: Float64 = -5.0\nputs x\nputs y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 19 (string literals).

  #[test]
  fn accepts_string_let_and_puts() {
    let src = "s: String = \"hello\"\nputs s\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_string_concat() {
    let src = "a: String = \"foo\" + \"bar\"\nputs a\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_string_plus_int() {
    let src = "puts \"foo\" + 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject String + Int64");
    assert!(errs[0].message.contains("String") && errs[0].message.contains("Int64"));
  }

  #[test]
  fn accepts_string_equality_compare() {
    let src = "if \"abc\" == \"abc\" do\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 20 (comments and case/when).

  const CASE_EXAMPLE: &str = "n: Int64 = 2\nlabel: Int64 = 0\nmatch n do\n  1 do  label: Int64 = 10\n  end\n  2, 3 do  label: Int64 = 20\n  end\n  _ do  label: Int64 = 99\n  end\nend\nputs label\n";

  #[test]
  fn accepts_case_when_example() {
    let program = emerald_parser::parse(CASE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_float64_case_scrutinee() {
    let src = "n: Float64 = 1.0\nmatch n do\n  1 do  puts 1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a Float64 scrutinee");
    assert!(errs[0].message.contains("Int64"));
  }

  // Plan 25 (stdlib expansion).

  #[test]
  fn accepts_bool_literal_example() {
    let src = "fn check(flag: Boolean): Int64 do\n  if flag do\n    return 1\n  end\n  return 0\nend\n\nputs check(true)\nputs check(false)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 73's Decision log: `nil`/`Nil` are removed outright, replaced
  // by `Option[T]`/`Some`/`None` — `accepts_nil_literal_example`'s own
  // premise (a bare `Nil`-typed parameter, a bare `nil` literal) has no
  // `Option[T]` analogue at all (`Nil` was a standalone, trivial type,
  // not a wrapper), so it is deleted rather than rewritten.
  #[test]
  fn rejects_a_some_construction_assigned_to_a_non_option_type() {
    // The direct `Option[T]` analogue of the old `x: Int64 = nil`
    // rejection: `Some(5)` is a real `Option[Int64]` construction, and
    // assigning it to a plain `Int64`-typed `Let` (not `Option[Int64]`)
    // must still be rejected as a type mismatch.
    let src = "x: Int64 = Some(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `x: Int64 = Some(5)`");
    assert!(errs[0].message.contains("Int64"), "{errs:?}");
  }

  #[test]
  fn accepts_hash_literal_get_and_set() {
    let src =
      "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\nputs h[2]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_empty_hash_literal() {
    let src = "h: Hash[Int64, Int64] = {}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty hash literal");
    assert!(errs[0].message.contains("empty hash literals"));
  }

  #[test]
  fn rejects_mixed_value_type_hash_literal() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 2.0}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a mixed value-type hash literal");
    assert!(errs[0].message.contains("value type"));
  }

  #[test]
  fn accepts_array_new_with_literal_and_runtime_size() {
    let src = "arr: Array[Int64] = Array.new(5)\nn: Int64 = 5\narr2: Array[Int64] = Array.new(n)\nputs arr[0]\nputs arr2[0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_array_new_size_mismatch_with_declared_type() {
    let src = "arr: Int64 = Array.new(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject Array.new assigned to a non-Array type");
    assert!(errs[0].message.contains("Array.new"));
  }

  // Plan 28 (bitwise operators).

  const BITWISE_EXAMPLE: &str = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\nfn has_flag(flags: Int64, flag: Int64): Boolean do\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ) do\n  puts 1\nend\nif has_flag(perms, EXEC) do\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";

  #[test]
  fn accepts_bitwise_example() {
    let program = emerald_parser::parse(BITWISE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_float64_bit_and_operand() {
    let src = "puts 2.0 & 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Float64 & Int64");
    assert!(errs[0].message.contains("&"));
    assert!(errs[0].message.contains("Int64") || errs[0].message.contains("Float64"));
  }

  #[test]
  fn rejects_float64_bit_not_operand() {
    let src = "puts ~2.5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `~2.5`");
    assert!(errs[0].message.contains("~"));
    assert!(errs[0].message.contains("Float64"));
  }

  #[test]
  fn bitwise_operators_type_check_as_int64() {
    let src = "a: Int64 = 1 << 2\nb: Int64 = 1 & 2\nc: Int64 = ~1\nputs a\nputs b\nputs c\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 30 (for-in iteration).

  #[test]
  fn accepts_for_in_with_var_usable_at_element_type() {
    let src = "for x in [1, 2, 3]\n  y: Int64 = x + 1\n  puts y\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_for_in_over_empty_array_literal() {
    // Grammar requires `[Args]`, and `Args` itself is grammar-legal
    // empty (`[]` parses fine) — this is a sema-level rejection, same
    // diagnostic as a bare `Expr::ArrayLit`.
    let src = "for x in []\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty for-in literal");
    assert!(errs[0].message.contains("empty array literals"));
  }

  #[test]
  fn rejects_for_in_over_heterogeneous_array_literal() {
    let src = "for x in [1, \"two\"]\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a heterogeneous for-in literal");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_break_and_next_inside_for_in() {
    let src = "for x in [1, 2, 3]\n  if x == 2 do\n    next\n  end\n  if x == 3 do\n    break\n  end\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 31 (compound and multiple assignment).

  #[test]
  fn accepts_bare_reassignment_of_an_existing_local() {
    let src = "var x: Int64 = 1\nx = 2\nputs x\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_reassignment_of_an_undeclared_local() {
    let src = "y = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject reassigning an undeclared local");
    assert!(errs[0].message.contains("undefined variable"));
  }

  #[test]
  fn rejects_reassignment_type_mismatch() {
    let src = "var x: Int64 = 1\nx = \"mismatched\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 reassigned to a String");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_compound_plus_assign_accumulator() {
    let src =
      "var total: Int64 = 0\nvar i: Int64 = 0\nwhile i < 5 do\n  total += i\n  i += 1\nend\nputs total\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_multiple_assignment_swap() {
    let src = "var a: Int64 = 1\nvar b: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_multiple_assignment_positional_type_mismatch() {
    let src = "var a: Int64 = 1\nvar b: Int64 = 2\na, b = \"s\", 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a positional type mismatch in `a, b = ...`");
    assert!(errs[0].message.contains("position 1"));
  }

  #[test]
  fn rejects_multiple_assignment_arity_mismatch() {
    let src = "var a: Int64 = 1\nvar b: Int64 = 2\na, b = 1, 2, 3\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an arity mismatch");
    assert!(errs[0].message.contains("arity mismatch"));
  }

  #[test]
  fn rejects_multiple_assignment_undefined_target() {
    let src = "a: Int64 = 1\na, z = 1, 2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undefined multi-assign target");
    assert!(errs[0].message.contains("undefined variable"));
  }

  // Plan 72 (immutable-by-default bindings).

  #[test]
  fn accepts_reassignment_of_a_var_declared_local() {
    let src = "var x: Int64 = 1\nx = 2\nputs x\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_reassignment_of_a_non_var_declared_local() {
    let src = "x: Int64 = 1\nx = 2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a binding declared without `var` must reject reassignment");
    assert!(
      errs[0]
        .message
        .contains("cannot reassign immutable binding `x`"),
      "expected the real immutable-binding diagnostic: {errs:?}"
    );
    assert!(errs[0].message.contains("declared without `var`"));
  }

  #[test]
  fn rejects_compound_assign_reassignment_of_a_non_var_declared_local() {
    // `+=` desugars to `Stmt::Assign` at parse time (plan 31's Decision
    // log) — this proves the immutability check fires against the
    // desugared form too, not just a literal `=`.
    let src = "x: Int64 = 1\nx += 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`+=` on a non-`var` binding must be rejected the same as a bare `=`");
    assert!(errs[0]
      .message
      .contains("cannot reassign immutable binding `x`"));
  }

  #[test]
  fn rejects_multiple_assignment_when_any_target_is_declared_without_var() {
    let src = "var a: Int64 = 1\nb: Int64 = 2\na, b = b, a\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`b` is declared without `var` — the whole multi-assign must be rejected");
    assert!(errs[0]
      .message
      .contains("cannot reassign immutable binding `b`"));
  }

  #[test]
  fn rejects_reassigning_a_function_parameter_without_var() {
    // Plan 72's Decision log: a parameter has no `var` slot in this
    // grammar at all — reassigning one inside its own function body is
    // unconditionally rejected, not silently accepted.
    let src = "fn f(x: Int64): Int64 do\n  x = x + 1\n  x\nend\n\nputs f(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("reassigning a parameter without `var` must be rejected");
    assert!(errs[0]
      .message
      .contains("cannot reassign immutable binding `x`"));
  }

  #[test]
  fn rejects_reassigning_a_for_loops_induction_variable() {
    // Plan 72's Decision log: `for`'s induction variable has no `var`
    // slot either — unconditionally immutable, the same rule as a
    // parameter.
    let src = "for i in [1, 2, 3]\n  i = i + 1\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("reassigning a `for` loop's own induction variable must be rejected");
    assert!(errs[0]
      .message
      .contains("cannot reassign immutable binding `i`"));
  }

  #[test]
  fn accepts_reassigning_an_instance_field_with_no_var_marker() {
    // Plan 72's Decision log: class instance fields (`@x`) are
    // deliberately exempt from this rule — every existing example
    // reassigns `@field` with no marker, and the grammar has no `var`
    // slot on a `ClassField` at all. `Stmt::SetField` is untouched by
    // this whole plan.
    let src = "class Counter\n  count: Int64\n\n  fn initialize: Void do\n    @count = 0\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\nend\n\nc: Counter = Counter.new()\nc.increment\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 32 (class inheritance).

  const INHERITANCE_EXAMPLE: &str = "class Animal\n  age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn age: Int64 do\n    @age\n  end\n\n  fn describe: Int64 do\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  fn initialize(age: Int64, breed_code: Int64): Void do\n    @age = age\n    @breed_code = breed_code\n  end\n\n  fn describe: Int64 do\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";

  #[test]
  fn accepts_inheritance_example() {
    let program = emerald_parser::parse(INHERITANCE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_cyclic_inheritance() {
    let src = "class A < B\n  x: Int64\nend\n\nclass B < A\n  y: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject cyclic inheritance");
    assert!(errs.iter().any(|e| e.message.contains("cyclic")));
  }

  #[test]
  fn rejects_field_redeclared_from_ancestor() {
    let src = "class Animal\n  age: Int64\nend\n\nclass Dog < Animal\n  age: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a subclass redeclaring an ancestor's field");
    assert!(errs[0].message.contains("age") && errs[0].message.contains("Animal"));
  }

  #[test]
  fn rejects_override_with_mismatched_signature() {
    let src = "class Animal\n  fn speak(volume: Int64): Int64 do\n    volume\n  end\nend\n\nclass Dog < Animal\n  fn speak(volume: Float64): Int64 do\n    1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject an override with a mismatched signature");
    assert!(errs[0].message.contains("speak"));
  }

  #[test]
  fn rejects_undeclared_superclass() {
    let src = "class Dog < NotAClass\n  x: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undeclared superclass");
    assert!(errs[0].message.contains("NotAClass"));
  }

  // Plan 33 (field-access sugar).

  const READ_FIELD_EXAMPLE: &str = "class Point\n  read x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";

  #[test]
  fn accepts_read_field_accessed_from_outside() {
    let program = emerald_parser::parse(READ_FIELD_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_read_field_accessed_from_outside() {
    // Opt-in per field, not blanket exposure: `y` has no `read` marker,
    // so `p.y` must be rejected with the same "no such method"
    // diagnostic plan 08 already produces for any undeclared method.
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `p.y` — `y` has no `read` marker");
    assert!(errs[0].message.contains('y'));
  }

  // Plan 34 (blocks and yield).

  const BLOCKS_EXAMPLE: &str = "fn repeat(n: Int64, &blk): Void do\n  i: Int64 = 0\n  while i < n do\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) do |i: Int64| puts i end\n";

  #[test]
  fn accepts_blocks_and_yield_example() {
    let program = emerald_parser::parse(BLOCKS_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_to_block_param_function_with_no_trailing_block() {
    let src = "fn repeat(n: Int64, &blk): Void do\n  yield n\nend\n\nrepeat(3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject calling a &blk function with no block");
    assert!(errs[0].message.contains("repeat") && errs[0].message.contains("block"));
  }

  #[test]
  fn rejects_block_arity_mismatch_against_yield() {
    let src = "fn repeat(n: Int64, &blk): Void do\n  i: Int64 = 0\n  while i < n do\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) do |i: Int64, extra: Int64| puts i end\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject a block whose arity doesn't match yield's call sites");
    assert!(errs[0].message.contains("arity"));
  }

  #[test]
  fn rejects_yield_outside_a_block_param_function() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  yield 5\n  a + b\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject `yield` in a function with no block parameter");
    assert!(errs[0].message.contains("yield"));
  }

  // Plan 23 (multi-file compilation).

  #[test]
  fn item_require_is_a_no_op_in_sema() {
    // `emerald-driver`'s resolution step (plan 17) isn't extracted in
    // this codebase yet, so a `Program` still containing `Item::
    // Require` is real, disclosed, in-scope input here — sema treats
    // it as a no-op rather than panicking or rejecting the whole
    // program.
    let src = "require helpers\nputs 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 36 (string interpolation and heredocs).

  #[test]
  fn accepts_interpolation_of_all_compiler_known_types() {
    let src = "s: String = \"a\"\nn: Int64 = 1\nf: Float64 = 2.5\nb: Boolean = true\nputs \"#{s} #{n} #{f} #{b}\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_interpolation_of_an_unsupported_type() {
    let src = "arr: Array[Int64] = [1]\nputs \"#{arr}\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject interpolating an Array — not a compiler-known stringifiable type");
    assert!(errs[0].message.contains("cannot be interpolated"));
  }

  // Plan 37 (ranges and range-based iteration).

  #[test]
  fn accepts_range_for_in_with_literal_endpoints() {
    let src = "for i in 1..5\n  x: Int64 = i + 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_range_for_in_with_non_literal_endpoint_expression() {
    // No parenthesized-expression grouping exists anywhere in this
    // grammar (verified: no `"(" Expr ")"` production, only call-arg
    // and param-list parens) — `n - 1` parses fine unparenthesized as
    // the range's end operand, the real proof endpoints are arbitrary
    // expressions, not just literals.
    let src = "n: Int64 = 5\nfor i in 0..n - 1\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_int64_range_endpoint() {
    let src = "for i in \"a\"..\"z\"\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a String range endpoint, not just Int64");
    assert!(errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_break_and_next_inside_a_range_for_in() {
    let src =
      "for i in 1..5\n  if i == 3 do\n    break\n  end\n  if i == 2 do\n    next\n  end\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 38 (full exception model).

  const FULL_EXCEPTION_EXAMPLE: &str = "class NotFoundError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nclass TimeoutError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise TimeoutError.new(7)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";

  #[test]
  fn accepts_the_full_worked_example_two_typed_rescues_and_ensure() {
    let program = emerald_parser::parse(FULL_EXCEPTION_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_referencing_a_bare_rescue_bound_name_inside_its_own_body() {
    let src = "begin\n  puts 1\nrescue => e\n  puts e\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a bare rescue's bound name must never enter env — no universal root class");
    assert!(errs[0].message.contains("undefined variable"));
  }

  #[test]
  fn rejects_retry_at_top_level() {
    let src = "retry\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject top-level retry");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn rejects_retry_inside_a_begin_try_body() {
    // A bare `rescue => e` avoids needing any class declared — this
    // test is purely about where `retry` is (il)legal, unrelated to a
    // typed clause's own class-resolution.
    let src = "begin\n  retry\nrescue => e\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject retry inside the begin's own try body, not just top level");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn rejects_retry_inside_an_ensure_body() {
    let src = "begin\n  puts 1\nrescue => e\n  puts 2\nensure\n  retry\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject retry inside an ensure body");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn accepts_retry_inside_a_rescue_clause_body() {
    let src = "begin\n  puts 1\nrescue => e\n  retry\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 39 (function signature completeness).

  #[test]
  fn accepts_call_omitting_a_defaulted_trailing_argument() {
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n\nputs inc(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_call_overriding_a_default_with_an_explicit_argument() {
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n\nputs inc(5, 10)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_omitting_a_required_no_default_argument() {
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n\nputs inc()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`n` has no default — must be required");
    assert!(errs[0].message.contains("missing required argument `n`"));
  }

  #[test]
  fn rejects_default_parameter_on_a_method() {
    let src = "class Foo\n  fn m(x: Int64 = 0): Int64 do\n    x\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("default parameter values are not supported on methods yet");
    assert!(errs[0].message.contains("not supported on methods"));
  }

  const GREET_EXAMPLE: &str = "fn greet(name: String, times: Int64 = 1): Void do\n  var i: Int64 = 0\n  while i < times do\n    puts name\n    i += 1\n  end\nend\n\ngreet(name: \"yo\")\ngreet(name: \"hi\", times: 2)\n";

  #[test]
  fn accepts_the_greet_worked_example_keyword_calls_and_defaults() {
    let program = emerald_parser::parse(GREET_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_misspelled_keyword_argument() {
    let src = "fn greet(name: String): Void do\n  puts name\nend\n\ngreet(nam: \"hi\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`nam` is not a declared parameter of greet");
    assert!(errs[0].message.contains("unrecognized keyword `nam`"));
  }

  #[test]
  fn rejects_call_missing_a_required_keyword() {
    // A bare `greet()` (no `name:` at all) would parse as an ordinary,
    // zero-arg `Expr::Call`, not `Expr::CallKw` — this test instead
    // supplies the defaulted keyword while omitting the required one,
    // to genuinely exercise `Expr::CallKw`'s own missing-keyword path.
    let src =
      "fn greet(name: String, times: Int64 = 1): Void do\n  puts name\nend\n\ngreet(times: 5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`name` has no default — must be required");
    assert!(errs[0].message.contains("missing required keyword `name`"));
  }

  #[test]
  fn rejects_duplicate_keyword_argument() {
    let src =
      "fn greet(name: String): Void do\n  puts name\nend\n\ngreet(name: \"hi\", name: \"yo\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a duplicate keyword");
    assert!(errs[0].message.contains("duplicate keyword `name`"));
  }

  const SUM_ALL_EXAMPLE: &str = "fn sum_all(*xs: Int64): Int64 do\n  var total: Int64 = 0\n  var i: Int64 = 0\n  while i < 4 do\n    total += xs[i]\n    i += 1\n  end\n  total\nend\n\nputs sum_all(1, 2, 3, 4)\n";

  #[test]
  fn accepts_splat_call_type_checking_trailing_arguments() {
    let program = emerald_parser::parse(SUM_ALL_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_splat_call_with_zero_trailing_arguments() {
    let src = "fn sum_all(*xs: Int64): Int64 do\n  0\nend\n\nputs sum_all()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_splat_call_with_a_mismatched_trailing_argument_type() {
    let src = "fn sum_all(*xs: Int64): Int64 do\n  0\nend\n\nputs sum_all(1, \"x\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a String trailing argument must not match Int64");
    assert!(errs[0].message.contains("trailing (splat) argument 2"));
  }

  #[test]
  fn rejects_splat_parameter_on_a_method() {
    let src = "class Foo\n  fn m(*xs: Int64): Int64 do\n    0\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("splat parameters are not supported on methods yet");
    assert!(errs[0].message.contains("not supported on methods"));
  }

  const DIVMOD_EXAMPLE: &str = "fn divmod(a: Int64, b: Int64): (Int64, Int64) do\n  return a / b, a % b\nend\n\nvar q: Int64 = 0\nvar r: Int64 = 0\nq, r = divmod(17, 5)\nputs q\nputs r\n";

  #[test]
  fn accepts_the_divmod_worked_example_tuple_return() {
    let program = emerald_parser::parse(DIVMOD_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_tuple_typed_let_annotation_as_an_unknown_type() {
    // `resolve_type` (used for every Let/param/field annotation) gets
    // no `TypeExpr::Tuple` branch — only `resolve_return_type` does. A
    // tuple is valid only as a function's declared return type. Plan
    // 88's Decision log: now a real, `TypeExpr`-aware diagnostic naming
    // the tuple shape directly, not the old generic "unknown type"
    // fallback the flat-string convention had to settle for.
    let src = "x: (Int64, Int64) = 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("a tuple type is not valid on a Let binding");
    assert!(errs[0].message.to_lowercase().contains("tuple type"));
  }

  #[test]
  fn rejects_tuple_return_type_on_a_method() {
    let src =
      "class Foo\n  fn m(a: Int64, b: Int64): (Int64, Int64) do\n    return a, b\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("tuple return types are not supported on methods yet");
    assert!(errs[0].message.contains("not supported on methods"));
  }

  // Plan 40 (operator overloading).

  const VECTOR2_EXAMPLE: &str = "class Vector2\n  read x: Float64\n  read y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn +(other: Vector2): Vector2 do\n    Vector2.new(@x + other.x, @y + other.y)\n  end\n\n  fn ==(other: Vector2): Boolean do\n    @x == other.x && @y == other.y\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0, 2.0)\nv2: Vector2 = Vector2.new(3.0, 4.0)\nv3: Vector2 = v1 + v2\nputs v3.x\nputs v3.y\nif v1 == v2 do\n  puts 1\nelse\n  puts 0\nend\nif v1 == v1 do\n  puts 1\nelse\n  puts 0\nend\n";

  #[test]
  fn accepts_the_vector2_worked_example() {
    let program = emerald_parser::parse(VECTOR2_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_plus_on_a_class_with_no_plus_method() {
    let src = "class Vector2\n  read x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nv3: Vector2 = v1 + v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Vector2 declares no `+` method");
    assert!(errs[0]
      .message
      .contains("class `Vector2` has no operator method `+`"));
  }

  #[test]
  fn rejects_operator_call_with_a_mismatched_argument_type() {
    let src = "class Vector2\n  read x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = x\n  end\n\n  fn +(other: Vector2): Vector2 do\n    Vector2.new(@x + other.x)\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = v1 + 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`+` expects a Vector2, not an Int64");
    assert!(errs[0].message.contains("argument 1 to `+`"));
  }

  #[test]
  fn rejects_eq_on_a_class_with_no_eq_method() {
    let src = "class Vector2\n  read x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nb: Boolean = v1 == v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Vector2 declares no `==` method");
    assert!(errs[0]
      .message
      .contains("class `Vector2` has no operator method `==`"));
  }

  #[test]
  fn rejects_ordering_comparison_on_a_class() {
    let src = "class Vector2\n  read x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nb: Boolean = v1 < v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("ordering comparisons on a class are not supported without `<=>`");
    assert!(errs[0].message.contains("ordering comparison"));
  }

  const BAG_EXAMPLE: &str = "class Bag\n  data: Array[Int64]\n\n  fn initialize(a: Int64, b: Int64, c: Int64): Void do\n    @data = [a, b, c]\n  end\n\n  fn [](i: Int64): Int64 do\n    @data[i]\n  end\n\n  fn []=(i: Int64, v: Int64): Void do\n    @data[i] = v\n  end\nend\n\nb: Bag = Bag.new(10, 20, 30)\nputs b[0] + b[1] + b[2]\nb[1] = 99\nputs b[1]\n";

  #[test]
  fn accepts_the_bag_index_operator_example_read_and_write() {
    let program = emerald_parser::parse(BAG_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_index_operator_with_a_mismatched_index_type() {
    let src = "class Bag\n  data: Array[Int64]\n\n  fn initialize(a: Int64): Void do\n    @data = [a]\n  end\n\n  fn [](i: Int64): Int64 do\n    @data[i]\n  end\nend\n\nb: Bag = Bag.new(1)\nx: Int64 = b[\"nope\"]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`[]` expects an Int64 index, not a String");
    assert!(errs[0].message.contains("argument 1 to `[]`"));
  }

  #[test]
  fn rejects_index_write_on_a_class_with_no_index_set_method() {
    let src = "class Bag\n  data: Array[Int64]\n\n  fn initialize(a: Int64): Void do\n    @data = [a]\n  end\n\n  fn [](i: Int64): Int64 do\n    @data[i]\n  end\nend\n\nb: Bag = Bag.new(1)\nb[0] = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Bag declares no `[]=` method");
    assert!(errs[0]
      .message
      .contains("class `Bag` has no operator method `[]=`"));
  }

  // Plan 41 (interfaces and generics).

  const COMPARABLE_MAX_EXAMPLE: &str = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Int64 do\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  fn initialize(meters: Int64): Void do\n    @meters = meters\n  end\n\n  fn compare_to(other: Distance): Int64 do\n    @meters - other.meters\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

  #[test]
  fn accepts_the_comparable_max_worked_example() {
    let program = emerald_parser::parse(COMPARABLE_MAX_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_implements_with_no_matching_method_defined() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Money never defines compare_to");
    assert!(errs[0].message.contains("Money"));
    assert!(errs[0].message.contains("Comparable"));
    assert!(errs[0].message.contains("compare_to"));
  }

  #[test]
  fn rejects_implements_with_a_mismatched_method_signature() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Boolean do\n    true\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("compare_to returns the wrong type");
    assert!(errs[0].message.contains("does not match interface"));
  }

  #[test]
  fn rejects_implements_naming_an_undefined_interface() {
    let src = "class Money implements NotAnInterface\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("NotAnInterface is never declared");
    assert!(errs[0].message.contains("NotAnInterface"));
  }

  #[test]
  fn accepts_conformance_satisfied_by_an_inherited_method() {
    // Real proof conformance is checked against the flattened method
    // table, not just the class's own declared methods (plan 32's
    // contribution) — `Dog` declares no `describe` of its own at all.
    // Uses a `Self`-free interface method deliberately: `Self`
    // substitutes to the *leaf* class declaring `implements` (invariant,
    // not covariant — see the Decision log), so an ancestor's own
    // `Self`-typed method (typed at the ancestor's own name) would
    // conflict with that leaf substitution on a completely separate
    // axis this AC isn't testing.
    let src = "interface Describable\n  fn describe(label: String): Int64\nend\n\nclass Animal\n  read age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn describe(label: String): Int64 do\n    @age\n  end\nend\n\nclass Dog < Animal implements Describable\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn generic_function_body_type_checks_once_against_the_bound_interface() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nfn describe[T: Comparable](a: T, b: T): Int64 do\n  a.compare_to(b)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_generic_call_with_inconsistent_type_parameter_arguments() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Int64 do\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  fn initialize(meters: Int64): Void do\n    @meters = meters\n  end\n\n  fn compare_to(other: Distance): Int64 do\n    @meters - other.meters\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nd1: Distance = Distance.new(100)\nboom: Money = max(m1, d1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`T` resolves to both Money and Distance");
    assert!(errs[0].message.contains("inconsistently"));
  }

  #[test]
  fn rejects_a_generic_call_whose_concrete_type_does_not_implement_the_bound() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Widget\n  read id: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nw1: Widget = Widget.new(1)\nw2: Widget = Widget.new(2)\nboom: Widget = max(w1, w2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Widget does not implement Comparable");
    assert!(errs[0].message.contains("does not implement"));
  }

  #[test]
  fn rejects_multiple_type_parameters_at_registration_time() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nfn bad[T: Comparable, U: Comparable](a: T, b: U): T do\n  a\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("two type parameters are not supported");
    assert!(errs[0]
      .message
      .contains("multiple type parameters are not supported"));
  }

  // Plan 88's Decision log: generic methods on a class are now real,
  // lifted from this exact "generic methods are not supported yet"
  // rejection (this test's own pre-plan-88 name/body, replaced rather
  // than deleted, since it's the direct proof the restriction is gone)
  // — reusing plan 41/58's existing top-level generic-function bare-
  // identifier-position monomorphization convention, extended to a
  // per-CLASS method table (`ClassInfo.generic_methods`).
  const GENERIC_METHOD_EXAMPLE: &str = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Widget\n  implements Comparable\n  read id: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n  end\n\n  fn compare_to(other: Widget): Int64 do\n    @id - other.id\n  end\nend\n\nclass Gadget\n  implements Comparable\n  read id: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n  end\n\n  fn compare_to(other: Gadget): Int64 do\n    @id - other.id\n  end\nend\n\nclass Box\n  fn initialize: Void do\n  end\n\n  fn pick[T: Comparable](a: T, b: T): T do\n    if a.compare_to(b) >= 0 do\n      return a\n    end\n    return b\n  end\nend\n\nbox: Box = Box.new()\nw1: Widget = Widget.new(1)\nw2: Widget = Widget.new(2)\npicked_widget: Widget = box.pick(w1, w2)\n\ng1: Gadget = Gadget.new(3)\ng2: Gadget = Gadget.new(4)\npicked_gadget: Gadget = box.pick(g1, g2)\n";

  #[test]
  fn a_generic_method_type_checks_and_monomorphizes_for_two_distinct_type_arguments() {
    let program = emerald_parser::parse(GENERIC_METHOD_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn a_generic_method_call_whose_concrete_type_does_not_implement_the_bound_is_rejected() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass NotComparable\n  read id: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n  end\nend\n\nclass Box\n  fn initialize: Void do\n  end\n\n  fn pick[T: Comparable](a: T, b: T): T do\n    if a.compare_to(b) >= 0 do\n      return a\n    end\n    return b\n  end\nend\n\nbox: Box = Box.new()\nn1: NotComparable = NotComparable.new(1)\nn2: NotComparable = NotComparable.new(2)\nboom: NotComparable = box.pick(n1, n2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("NotComparable does not implement Comparable");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("does not implement")));
  }

  #[test]
  fn a_multi_bound_generic_function_rejects_a_type_argument_satisfying_only_one_bound() {
    // Plan 88's Decision log: `[T: Comparable + Cloneable]` — a real
    // bound SET (absorbing plan 75's scope). `OnlyComparable` satisfies
    // the FIRST bound but not the second, so this must still be
    // rejected — every bound in the set is required, not just one.
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\ninterface Cloneable\n  fn duplicate(other: Self): Int64\nend\n\nclass OnlyComparable\n  implements Comparable\n  read id: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n  end\n\n  fn compare_to(other: OnlyComparable): Int64 do\n    @id - other.id\n  end\nend\n\nfn pick[T: Comparable + Cloneable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\no1: OnlyComparable = OnlyComparable.new(1)\no2: OnlyComparable = OnlyComparable.new(2)\nboom: OnlyComparable = pick(o1, o2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("OnlyComparable does not implement Cloneable, the second bound");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("does not implement `Cloneable`")));
  }

  // Plan 89 (interface generics, multi-method interfaces).

  const ITERABLE_WORKED_EXAMPLE: &str = "interface Iterable[T]\n  fn map[U](f: Proc[T, U]): Array[U]\nend\n\nclass Numbers\n  implements Iterable[Int64]\n\n  values: Array[Int64]\n\n  fn initialize(values: Array[Int64]): Void do\n    @values = values\n  end\n\n  fn map[U](f: Proc[T, U]): Array[U] do\n    values: Array[Int64] = @values\n    result: Array[U] = Array.new(values.count)\n    i: Int64 = 0\n    while i < values.count do\n      result[i] = f.call(values[i])\n      i: Int64 = i + 1\n    end\n    result\n  end\nend\n\nn: Numbers = Numbers.new([1, 2, 3])\ndoubled: Array[Int64] = n.map(do |x: Int64| x * 2 end)\nputs doubled[0]\nputs doubled[2]\n";

  #[test]
  fn interface_generics_worked_example_type_checks() {
    let program = emerald_parser::parse(ITERABLE_WORKED_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn implements_a_generic_interface_with_the_wrong_arity_is_rejected() {
    let src = "interface Iterable[T]\n  fn map[U](f: Proc[T, U]): Array[U]\nend\n\nclass Numbers\n  implements Iterable\n\n  values: Array[Int64]\n\n  fn initialize(values: Array[Int64]): Void do\n    @values = values\n  end\n\n  fn map[U](f: Proc[Int64, U]): Array[U] do\n    result: Array[U] = Array.new(0)\n    result\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("arity mismatch between implements and interface");
    assert!(errs[0].message.contains("type argument"));
  }

  #[test]
  fn a_class_missing_a_required_generic_method_is_rejected() {
    let src = "interface Iterable[T]\n  fn map[U](f: Proc[T, U]): Array[U]\nend\n\nclass Numbers\n  implements Iterable[Int64]\n\n  values: Array[Int64]\n\n  fn initialize(values: Array[Int64]): Void do\n    @values = values\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Numbers never defines map");
    assert!(errs[0]
      .message
      .contains("does not define required generic method"));
  }

  #[test]
  fn nested_proc_type_annotation_resolves_to_a_real_structured_proc_type() {
    // Plan 88's own worked proof: `Proc[Array[Int64], Int64]` — a
    // written Proc annotation with NO adjacent `Expr::Lambda` anywhere
    // (a plain function parameter) — resolves directly to a real,
    // structurally nested `Type::Proc`, round-tripping through parsing
    // (`TypeExpr::Func`) and `resolve_type` correctly. Genuinely
    // impossible before this plan: `Proc` had no written, parameterized
    // form at all (plan 10's own decision log), and even if it had,
    // the old flat-string convention couldn't nest `Array[Int64]`
    // inside another compound type's own bracket slot.
    let src =
      "fn apply(f: Proc[Array[Int64], Int64], xs: Array[Int64]): Int64 do\n  f.call(xs)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    let classes = HashMap::new();
    let resolved = resolve_type(&f.params[0].ty, &classes).expect("should resolve");
    assert_eq!(
      resolved,
      Type::Proc(
        vec![Type::Array(Box::new(Type::Int64))],
        Box::new(Type::Int64)
      )
    );
  }

  #[test]
  fn a_generic_method_declaring_two_type_parameters_is_rejected_at_registration_time() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Box\n  fn initialize: Void do\n  end\n\n  fn bad[T: Comparable, U: Comparable](a: T, b: U): T do\n    a\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("two type parameters are not supported");
    assert!(errs.iter().any(|d| d
      .message
      .contains("multiple type parameters are not supported")));
  }

  // Plan 43 (nullable types and safe navigation) — replaced outright by
  // plan 73's `Option[T]`/`Some`/`None`/`?.`/`??`.

  const GREETER_PREFIX: &str = "class Greeter\n  name: String\n\n  fn initialize(name: String): Void do\n    @name = name\n  end\n\n  fn shout: String do\n    @name + \"!\"\n  end\nend\n\nfn find_greeter(id: Int64): Option[Greeter] do\n  if id == 1 do\n    return Some(Greeter.new(\"ada\"))\n  end\n  return None\nend\n\n";

  const NULLABLE_WORKED_EXAMPLE: &str = "class Greeter\n  name: String\n\n  fn initialize(name: String): Void do\n    @name = name\n  end\n\n  fn shout: String do\n    @name + \"!\"\n  end\nend\n\nfn find_greeter(id: Int64): Option[Greeter] do\n  if id == 1 do\n    return Some(Greeter.new(\"ada\"))\n  end\n  return None\nend\n\nfn greet(id: Int64): String do\n  g: Option[Greeter] = find_greeter(id)\n  message: String = g?.shout ?? \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";

  #[test]
  fn accepts_the_nullable_worked_example() {
    let program = emerald_parser::parse(NULLABLE_WORKED_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_a_real_class_value_and_none_both_widening_into_an_option_let() {
    let src = format!(
      "{GREETER_PREFIX}g1: Option[Greeter] = Some(Greeter.new(\"ada\"))\ng2: Option[Greeter] = None\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // `rejects_a_nullable_value_type_annotation` (the old `y: Int64? = 5`
  // rejection) is deleted, not rewritten — `Int64?` doesn't parse at
  // all anymore (plan 73 removes `T?` outright), and the restriction it
  // tested (nullable VALUE types are unsupported) genuinely no longer
  // exists under `Option[T]`: `Option[Int64]` is an ordinary generic-
  // enum instantiation over a value-typed field, the same shape
  // `Result[T, E]`/user `enum`s already support (see e.g. plan 52's own
  // `Rectangle(Float64, Float64)`), not a pointer-nullness sentinel
  // that only ever worked for pointer-representable `T`.

  #[test]
  fn rejects_a_direct_method_call_on_an_option_receiver() {
    let src = format!(
      "{GREETER_PREFIX}fn greet(id: Int64): String do\n  g: Option[Greeter] = find_greeter(id)\n  return g.shout\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err("g is Option[Greeter], .shout is unguarded");
    assert!(errs[0].message.contains("Option"), "{errs:?}");
  }

  #[test]
  fn option_match_on_some_and_none_type_checks_to_boolean() {
    // The `Option[T]` analogue of the old `g == nil` truthiness check —
    // `==`/`!=` are not supported on enum-typed operands at all (only
    // `match` distinguishes `Some`/`None`), so this exercises the real
    // replacement mechanism instead of a removed comparison operator.
    let src = format!(
      "{GREETER_PREFIX}fn is_missing(id: Int64): Boolean do\n  g: Option[Greeter] = find_greeter(id)\n  var result: Boolean = false\n  match g do\n    Some(v) do\n      result = false\n    end\n    None do\n      result = true\n    end\n  end\n  return result\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn safe_call_on_an_option_class_receiver_type_checks_to_the_wrapped_return_type() {
    let src = format!(
      "{GREETER_PREFIX}fn greet(id: Int64): String do\n  g: Option[Greeter] = find_greeter(id)\n  message: String = g?.shout ?? \"nobody here\"\n  return message\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_safe_call_on_a_non_option_receiver() {
    let src = "class Greeter\n  name: String\n\n  fn initialize(name: String): Void do\n    @name = name\n  end\n\n  fn shout: String do\n    @name + \"!\"\n  end\nend\n\nfn greet: Option[String] do\n  g: Greeter = Greeter.new(\"ada\")\n  return g?.shout\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("g is never Option[T], ?. is illegal");
    assert!(errs[0].message.contains("Option[T]"), "{errs:?}");
  }

  #[test]
  fn rejects_safe_call_on_a_method_returning_a_value_type_with_no_prior_option_instantiation() {
    // `?.`'s ret-rewrap requires the resulting `Option[U]` to already be
    // instantiated somewhere else in the program (Decision log) — a
    // bare `g?.years` with no `Option[Int64]` annotated anywhere else
    // is genuinely rejected, the direct `Option[T]` descendant of the
    // old "Int64 is not pointer-representable" rejection.
    let src = "class Greeter\n  age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn years: Int64 do\n    @age\n  end\nend\n\nfn find_greeter(id: Int64): Option[Greeter] do\n  return None\nend\n\nfn ages(id: Int64): Int64 do\n  g: Option[Greeter] = find_greeter(id)\n  g?.years\n  return 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Option[Int64] is never instantiated");
    assert!(errs.iter().any(|d| d.message.contains("Int64")), "{errs:?}");
  }

  #[test]
  fn coalesce_narrows_the_tracked_type_so_a_later_return_type_checks() {
    // The `Option[T]` analogue of the old `||=`-narrowing test: `??`
    // itself (not a two-step reassignment) is what narrows `Option[T]`
    // down to a plain `T`, so `return message` type-checks against the
    // enclosing `String`-returning function.
    let src = format!(
      "{GREETER_PREFIX}fn greet(id: Int64): String do\n  g: Option[Greeter] = find_greeter(id)\n  message: String = g?.shout ?? \"nobody here\"\n  return message\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // `accepts_the_and_assign_upgrade_worked_example`/`UPGRADE_EXAMPLE`
  // (plan 43's `&&=`, "assign only if the target is currently non-nil")
  // and `rejects_and_assign_on_a_non_nullable_target` are deleted, not
  // rewritten — plan 73's Decision log replaces `&.`/`||=` with `?.`/
  // `??` but introduces no `&&=`-shaped operator at all, so there is no
  // `Option[T]` construct left for either test to exercise.

  #[test]
  fn rejects_coalesce_on_a_non_option_target() {
    let src = "s: String = \"x\"\nt: String = s ?? \"y\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("s is not Option[T]");
    assert!(errs[0].message.contains("Option[T]"), "{errs:?}");
  }

  #[test]
  fn rejects_coalesce_default_that_is_itself_an_option() {
    let src =
      "message: Option[String] = None\nother: Option[String] = None\nresult: String = message ?? other\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  // Plan 44 (symbols).

  #[test]
  fn accepts_a_symbol_typed_let() {
    let src = "x: Symbol = :foo\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn symbol_equality_and_inequality_both_type_check_to_boolean() {
    let src = "if :foo == :foo do\n  puts 1\nelse\n  puts 0\nend\nif :foo == :bar do\n  puts 1\nelse\n  puts 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_symbol_compared_against_string() {
    let src = "if :foo == \"foo\" do\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Symbol is not String");
    assert!(errs[0].message.contains("Symbol"));
    assert!(errs[0].message.contains("String"));
  }

  #[test]
  fn rejects_int64_literal_assigned_to_a_symbol_typed_let() {
    let src = "x: Symbol = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn accepts_a_symbol_keyed_hash_literal_matching_its_declared_annotation() {
    let src = "h: Hash[Symbol, Int64] = {:a => 1, :b => 2}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_the_symbols_worked_example() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo do\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar do\n  puts 1\nelse\n  puts 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 45 (stdlib strings and I/O).

  #[test]
  fn accepts_all_six_basic_string_intrinsics() {
    let src = "s: String = \"chicago\"\nn: Int64 = s.length\nu: String = s.upcase\nd: String = s.downcase\nt: String = s.strip\ni: Int64 = s.to_i\nf: Float64 = s.to_f\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_string_intrinsic_name_on_a_non_string_receiver() {
    let src = "x: Int64 = 5\ny: Int64 = x.length\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Int64 has no .length");
    assert!(errs[0].message.contains("Int64"));
  }

  #[test]
  fn rejects_a_string_intrinsic_called_with_the_wrong_arity() {
    let src = "s: String = \"hi\"\nu: String = s.upcase(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn accepts_string_indexing_and_slicing() {
    let src = "s: String = \"hello\"\nc: String = s[1]\nsub: String = s.slice(1, 3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_float_index_into_a_string() {
    let src = "s: String = \"hello\"\nidx: Float64 = 1.0\nc: String = s[idx]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn rejects_slice_called_with_the_wrong_arity() {
    let src = "s: String = \"hello\"\nc: String = s.slice(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn accepts_split_and_split_count() {
    let src = "s: String = \"a b c\"\nn: Int64 = s.split_count(\" \")\nwords: Array[String] = s.split(\" \")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_file_read_and_write() {
    let src = "File.write(\"x.txt\", \"hello\")\ncontent: String = File.read(\"x.txt\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_file_read_with_an_int64_argument() {
    let src = "content: String = File.read(42)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn rejects_file_write_with_only_one_argument() {
    let src = "File.write(\"only-one-arg.txt\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn accepts_argv_and_argc_and_gets() {
    let src = "puts ARGC\nfirst: String = ARGV[0]\nline: String = gets()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_the_plan_45_worked_example() {
    let src = "input: String = \"hello world foo\"\nupper: String = input.upcase\nFile.write(\"plan45_demo.txt\", upper)\nreadback: String = File.read(\"plan45_demo.txt\")\nputs readback\nn: Int64 = readback.split_count(\" \")\nputs n\nwords: Array[String] = readback.split(\" \")\nvar i: Int64 = 0\nwhile i < n do\n  puts words[i]\n  i += 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 47 (REPL and test framework).

  #[test]
  fn accepts_a_test_block_using_assert_and_assert_eq() {
    let src = "test \"addition works\" do\n  assert(true)\n  assert_eq(2, 1 + 1)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_assert_with_a_non_boolean_condition() {
    let src = "test \"bad\" do\n  assert(1)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn rejects_assert_eq_with_mismatched_operand_types() {
    // AC4: rejected with the existing Compare-family type-mismatch
    // diagnostic (naming String/Int64), not a runtime failure.
    let src = "test \"bad\" do\n  assert_eq(\"s\", 1)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("String vs Int64 assert_eq should be rejected");
    assert!(errs[0].message.contains("String"));
    assert!(errs[0].message.contains("Int64"));
  }

  #[test]
  fn accepts_a_top_level_assert_outside_any_test_block() {
    // `assert`/`assert_eq` are ordinary recognized-call-name intrinsics
    // (like `puts`) — legal anywhere an `Expr::Call` is, not just
    // inside `test ... do ... end`.
    let src = "assert(1 + 1 == 2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 21 (LSP symbols and navigation).

  #[test]
  fn collect_symbols_on_hello_em_finds_add() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let table = collect_symbols(&program);
    let add = table
      .functions
      .get("add")
      .expect("`add` should be in the table");
    assert_eq!(add.params, vec![Type::Int64, Type::Int64]);
    assert_eq!(add.return_type, Type::Int64);
  }

  #[test]
  fn collect_symbols_on_classes_em_finds_both_classes_and_their_methods() {
    let src = "class Counter\n  value: Int64\n\n  fn initialize(start: Int64): Void do\n    @value = start\n  end\n\n  fn value: Int64 do\n    @value\n  end\n\n  fn add(n: Int64): Int64 do\n    @value + n\n  end\nend\n\nclass Point\n  x: Float64\n  y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn sum: Float64 do\n    @x + @y\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let table = collect_symbols(&program);

    let counter = table
      .classes
      .get("Counter")
      .expect("Counter should be in the table");
    assert!(!counter.is_module);
    assert_eq!(counter.fields.get("value"), Some(&Type::Int64));
    assert!(counter.methods.contains_key("initialize"));
    assert!(counter.methods.contains_key("value"));
    assert!(counter.methods.contains_key("add"));

    let point = table
      .classes
      .get("Point")
      .expect("Point should be in the table");
    assert_eq!(point.fields.get("x"), Some(&Type::Float64));
    assert_eq!(point.fields.get("y"), Some(&Type::Float64));
    assert!(point.methods.contains_key("initialize"));
    assert!(point.methods.contains_key("sum"));
  }

  #[test]
  fn collect_symbols_omits_only_the_class_with_an_unresolvable_field_type() {
    // AC3: the degrade is per-declaration, not all-or-nothing.
    let src = "class Bad\n  x: NoSuchType\nend\n\nclass Good\n  y: Int64\nend\n\nfn ok(): Int64 do\n  1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let table = collect_symbols(&program);
    assert!(!table.classes.contains_key("Bad"));
    assert!(table.classes.contains_key("Good"));
    assert!(table.functions.contains_key("ok"));
  }

  // Plan 22 (sema diagnostic spans) — leaf-sema-spans.

  #[test]
  fn add_type_mismatch_diagnostic_span_points_at_the_real_second_b() {
    // AC1: the exact concrete-proof program from this plan's own
    // Decision log — proves the span actually threads from the parser
    // through sema (not just that the AST carries one internally).
    let src = "fn add(a: Int64, b: String): Int64 do\n  a + b\nend";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 + String");
    assert_eq!(errs.len(), 1);
    let second_b = src
      .match_indices('b')
      .nth(1)
      .expect("source has two occurrences of 'b'")
      .0;
    assert_eq!(errs[0].span, (second_b, second_b + 1));
  }

  #[test]
  fn arity_mismatch_diagnostic_span_covers_the_whole_call_expression() {
    // AC2: `add(20)`'s own span — not just `add` and not just `20`.
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 1-arg call to 2-arg add");
    let d = errs
      .iter()
      .find(|d| d.message.contains("missing required argument `b`"))
      .expect("expected the missing-argument diagnostic");
    let call_start = src.find("add(20)").unwrap();
    assert_eq!(d.span, (call_start, call_start + "add(20)".len()));
  }

  #[test]
  fn break_outside_loop_diagnostic_span_is_the_break_statements_own_span() {
    // AC3: the honest fallback — no better sub-expression to blame,
    // so the statement's own span, still real and non-degenerate.
    let src = "break\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject break outside a loop");
    assert_eq!(errs[0].span, (0, "break".len()));
  }

  // Plan 52 (algebraic data types and exhaustive pattern matching).

  const SHAPE_ENUM_WORKED_EXAMPLE: &str = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nsquare: Shape = Square(3.0)\nrect: Shape = Rectangle(4.0, 5.0)\n\nvar area: Float64 = 0.0\nmatch circle do\n  Circle(r) do  area = 3.14159 * r * r\n  end\n  Square(s) do  area = s * s\n  end\n  Rectangle(w, h) do  area = w * h\n  end\nend\nputs area\n\nmatch square do\n  Circle(r) do  area = 3.14159 * r * r\n  end\n  Square(s) do  area = s * s\n  end\n  Rectangle(w, h) do  area = w * h\n  end\nend\nputs area\n\nmatch rect do\n  Circle(r) do  area = 3.14159 * r * r\n  end\n  Square(s) do  area = s * s\n  end\n  Rectangle(w, h) do  area = w * h\n  end\nend\nputs area\n";

  #[test]
  fn accepts_the_shape_worked_example() {
    let program = emerald_parser::parse(SHAPE_ENUM_WORKED_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_case_missing_a_variant_naming_it_specifically() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nvar area: Float64 = 0.0\nmatch circle do\n  Circle(r) do  area = r\n  end\n  Square(s) do  area = s\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a non-exhaustive case");
    assert!(
      errs.iter().any(|d| d.message.contains("Rectangle")),
      "expected a diagnostic naming Rectangle specifically: {errs:?}"
    );
    assert!(
      !errs
        .iter()
        .any(|d| d.message.to_lowercase().contains("non-exhaustive")),
      "must not use a generic non-exhaustive message: {errs:?}"
    );
  }

  #[test]
  fn rejects_an_unknown_variant_pattern() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nvar area: Float64 = 0.0\nmatch circle do\n  Circle(r) do  area = r\n  end\n  Square(s) do  area = s\n  end\n  Rectangle(w, h) do  area = w\n  end\n  Triangle(a, b, c) do  area = a\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Triangle is not a variant of Shape");
    assert!(
      errs.iter().any(|d| d.message.contains("Triangle")),
      "expected a diagnostic naming Triangle: {errs:?}"
    );
  }

  #[test]
  fn rejects_a_duplicate_variant_arm() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nvar area: Float64 = 0.0\nmatch circle do\n  Circle(r) do  area = r\n  end\n  Circle(r2) do  area = r2\n  end\n  Square(s) do  area = s\n  end\n  Rectangle(w, h) do  area = w\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Circle matched twice");
    assert!(
      errs.iter().any(|d| d.message.contains("more than once")),
      "expected a duplicate-arm diagnostic: {errs:?}"
    );
  }

  #[test]
  fn rejects_an_arity_mismatched_pattern() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\nrect: Shape = Rectangle(4.0, 5.0)\nvar area: Float64 = 0.0\nmatch rect do\n  Circle(r) do  area = r\n  end\n  Square(s) do  area = s\n  end\n  Rectangle(w) do  area = w\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Rectangle needs two bindings, not one");
    assert!(
      errs.iter().any(|d| d.message.contains("expects 2 binding")),
      "expected an arity diagnostic naming expected vs. actual: {errs:?}"
    );
  }

  #[test]
  fn rejects_a_type_mismatched_construction() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(\"not a float\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Circle expects a Float64, not a String");
    assert!(!errs.is_empty());
  }

  #[test]
  fn rejects_a_pattern_binding_referenced_outside_its_own_arm() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nvar area: Float64 = 0.0\nmatch circle do\n  Circle(r) do  area = r\n  end\n  Square(s) do  area = r\n  end\n  Rectangle(w, h) do  area = w\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`r` is Circle's own binding, not visible inside the Square arm");
    assert!(
      errs
        .iter()
        .any(|d| d.message.to_lowercase().contains("undefined")),
      "expected an undefined-variable diagnostic: {errs:?}"
    );
  }

  #[test]
  fn rejects_a_pattern_binding_referenced_after_the_case_ends() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nmatch circle do\n  Circle(r) do  puts r\n  end\n  Square(s) do  puts s\n  end\n  Rectangle(w, h) do  puts w\n  end\nend\nputs r\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`r` does not survive past its own arm");
    assert!(
      errs
        .iter()
        .any(|d| d.message.to_lowercase().contains("undefined")),
      "expected an undefined-variable diagnostic: {errs:?}"
    );
  }

  #[test]
  fn rejects_two_enums_declaring_the_same_variant_name() {
    let src = "enum A = Empty(Int64)\nenum B = Empty(Int64)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Empty is declared by both A and B");
    assert!(errs.iter().any(|d| d.message.contains("Empty")));
  }

  #[test]
  fn rejects_a_variant_name_colliding_with_an_existing_function() {
    let src =
      "fn Circle(x: Int64): Int64 do\n  x\nend\n\nenum Shape = Circle(Float64) | Square(Float64)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("variant Circle collides with the function Circle");
    assert!(errs.iter().any(|d| d.message.contains("Circle")));
  }

  #[test]
  fn rejects_variant_pattern_over_an_int64_scrutinee() {
    let src = "n: Int64 = 1\nmatch n do\n  Circle(r) do  puts r\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Int64 scrutinee can't use a variant pattern");
    assert!(!errs.is_empty());
  }

  #[test]
  fn rejects_value_pattern_over_an_enum_scrutinee() {
    let src = "enum Shape = Circle(Float64)\n\ncircle: Shape = Circle(2.0)\nmatch circle do\n  1 do  puts 1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("enum scrutinee can't use a value pattern");
    assert!(!errs.is_empty());
  }

  // Plan 53 (Result type and error propagation).

  #[test]
  fn resolve_type_parses_result_t_e_directly() {
    let classes = HashMap::new();
    let texpr = TypeExpr::Generic(
      "Result".to_string(),
      vec![
        TypeExpr::Named("Int64".to_string()),
        TypeExpr::Named("String".to_string()),
      ],
    );
    let ty = resolve_type(&texpr, &classes).expect("should resolve");
    assert_eq!(
      ty,
      Type::Result(Box::new(Type::Int64), Box::new(Type::String))
    );
  }

  const RESULT_WORKED_EXAMPLE: &str = "fn parse_int(s: String): Result[Int64, String] do\n  if is_valid_int(s) do\n    return Ok(parse_digits(s))\n  end\n  return Err(\"not a number\")\nend\n\nfn try_parse(s: String): Result[Int64, String] do\n  n: Int64 = parse_int(s)?\n  return Ok(n * 2)\nend\n\nresult: Result[Int64, String] = try_parse(\"21\")\nmatch result do\n  Ok(v) do  puts v\n  end\n  Err(e) do  puts e\n  end\nend\n";

  #[test]
  fn accepts_the_result_worked_example() {
    let program = emerald_parser::parse(RESULT_WORKED_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_ok_with_the_wrong_t() {
    let src = "fn f: Result[Int64, String] do\n  return Ok(\"wrong\")\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Ok(\"wrong\") does not match declared T=Int64");
    assert!(errs.iter().any(|d| d.message.contains("Ok")));
  }

  #[test]
  fn rejects_err_with_the_wrong_e() {
    let src = "fn f: Result[Int64, String] do\n  return Err(42)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Err(42) does not match declared E=String");
    assert!(errs.iter().any(|d| d.message.contains("Err")));
  }

  #[test]
  fn rejects_ok_used_as_a_bare_call_argument() {
    let src = "fn f(x: Int64): Int64 do\n  x\nend\n\nputs f(Ok(1))\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("Ok(1) with no enclosing expected-type position can't infer T/E");
    assert!(errs
      .iter()
      .any(|d| d.message.to_lowercase().contains("infer")));
  }

  #[test]
  fn rejects_try_whose_e_does_not_match_the_enclosing_return_type() {
    let src = "class IoError\nend\n\nfn parse_int(s: String): Result[Int64, String] do\n  return Err(\"bad\")\nend\n\nfn try_parse(s: String): Result[Int64, IoError] do\n  n: Int64 = parse_int(s)?\n  return Ok(IoError.new())\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("Result[Int64, String]? inside a Result[Int64, IoError] function — E mismatch");
    assert!(errs
      .iter()
      .any(|d| d.message.to_lowercase().contains("error type")));
  }

  #[test]
  fn rejects_try_outside_a_result_returning_function() {
    let src = "fn parse_int(s: String): Result[Int64, String] do\n  return Err(\"bad\")\nend\n\nn: Int64 = parse_int(\"x\")?\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`?` at the top level has no Result-typed return_type");
    assert!(!errs.is_empty());
  }

  #[test]
  fn rejects_try_used_outside_a_let_or_assign_value() {
    let src = "fn parse_int(s: String): Result[Int64, String] do\n  return Err(\"bad\")\nend\n\nfn try_parse(s: String): Result[Int64, String] do\n  return Ok(1 + parse_int(s)?)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`?` nested inside a binary operator is not supported");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("let") || d.message.contains("assignment")));
  }

  #[test]
  fn accepts_ok_err_match_form_with_correctly_typed_bindings() {
    let src = "fn parse_int(s: String): Result[Int64, String] do\n  return Err(\"bad\")\nend\n\nresult: Result[Int64, String] = parse_int(\"x\")\nmatch result do\n  Ok(v) do  n: Int64 = v\n  puts n\n  end\n  Err(e) do  s: String = e\n  puts s\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_ok_binding_referenced_in_the_err_arm() {
    let src = "fn parse_int(s: String): Result[Int64, String] do\n  return Err(\"bad\")\nend\n\nresult: Result[Int64, String] = parse_int(\"x\")\nmatch result do\n  Ok(v) do  puts v\n  end\n  Err(e) do  puts v\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`v` is bound only inside the Ok arm, not visible in the Err arm");
    assert!(errs
      .iter()
      .any(|d| d.message.to_lowercase().contains("undefined")));
  }

  #[test]
  fn rejects_match_result_over_a_non_result_scrutinee() {
    let src =
      "n: Int64 = 1\nmatch n do\n  Ok(v) do  puts v\n  end\n  Err(e) do  puts e\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("an Int64 scrutinee is not Result[T, E]-typed");
    assert!(!errs.is_empty());
  }

  // Plan 54 (actor declarations and isolated heaps).

  // Plan 55's Decision log tightened this worked example (originally
  // authored under plan 54, before that rule existed): an actor
  // method other than `initialize` may no longer declare a return
  // type — `value` now prints `@count` itself (`puts @count`) rather
  // than returning it for a top-level `puts a.value` to print.
  const COUNTER_ACTOR_EXAMPLE: &str = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n\na.increment\na.increment\nb.increment\n\na.value\nb.value\n";

  #[test]
  fn accepts_the_actor_worked_example() {
    let program = emerald_parser::parse(COUNTER_ACTOR_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_new_called_on_an_actor() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\nc: Counter = Counter.new(0)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("actors are constructed with `.spawn`, not `.new`");
    assert!(errs.iter().any(|d| d.message.contains(".spawn")));
  }

  #[test]
  fn rejects_spawn_called_on_a_plain_class() {
    let src = "class Foo\nend\n\nf: Foo = Foo.spawn()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`.spawn` only constructs actors, and Foo is not one");
    assert!(errs.iter().any(|d| d.message.contains(".spawn")));
  }

  #[test]
  fn rejects_spawn_with_wrong_arity() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\nc: Counter = Counter.spawn(0, 1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`initialize` takes exactly one argument");
    assert!(!errs.is_empty());
  }

  #[test]
  fn rejects_spawn_with_wrong_argument_type() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\nc: Counter = Counter.spawn(\"x\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`initialize` expects Int64, not String");
    assert!(!errs.is_empty());
  }

  // Plan 55 (scheduler and message passing).

  #[test]
  fn rejects_a_non_initialize_actor_method_declaring_a_return_type() {
    let src = "actor Counter\n  count: Int64\n\n  fn get: Int64 do\n    @count\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("cross-actor calls are asynchronous — a method can't return a value");
    assert!(errs
      .iter()
      .any(|d| d.message.contains("get") && d.message.contains("return type")));
  }

  #[test]
  fn initialize_is_exempt_from_the_no_return_type_rule_in_either_direction() {
    // "exactly one exception" (the plan's own Decision log wording):
    // `initialize` is accepted whether it declares `Void` (the grammar's
    // own "no return type" default) or a real value type — unlike
    // every other actor method, which is unconditionally rejected for
    // declaring anything but `Void`.
    let void_src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\nc: Counter = Counter.spawn(0)\n";
    let program = emerald_parser::parse(void_src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));

    let value_src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Int64 do\n    @count = start\n    start\n  end\nend\n";
    let program = emerald_parser::parse(value_src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "`initialize` may declare a real return type — `.spawn` calls it directly and \
       synchronously, before any mailbox exists"
    );
  }

  #[test]
  fn accepts_every_non_initialize_method_declaring_no_return_type() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn bump: Void do\n    @count = @count + 1\n  end\nend\n\nc: Counter = Counter.spawn(0)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 56 (compile-time message safety).

  /// Shared class/actor preamble for every scenario below — `run_body`
  /// supplies just the varying statements. `Logger#ping` (a value-type
  /// param) backs the value-type-argument tests; `Logger#log` (a
  /// `LogMessage`-typed param) backs the reference-type ones.
  fn message_safety_program(run_body: &str) -> String {
    format!(
      "class LogMessage\n  text: String\n\n  fn initialize(text: String): Void do\n    @text = text\n  end\n\n  fn text: String do\n    @text\n  end\nend\n\nactor Logger\n  fn log(msg: LogMessage): Void do\n    puts msg.text\n  end\n\n  fn ping(n: Int64): Void do\n  end\nend\n\nfn run: Void do\n{run_body}end\n\nrun()\n"
    )
  }

  #[test]
  fn accepts_the_message_safety_worked_example() {
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  logger.log(msg)\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_read_of_a_local_immediately_after_sending_it() {
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  logger.log(msg)\n  puts msg.text\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`msg` was already sent to Logger — reading it again must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("message-safety") && d.message.contains("msg")),
      "diagnostic must be distinguishable from an ordinary undefined-variable/type-mismatch \
       error: {errs:?}"
    );
  }

  #[test]
  fn accepts_a_send_in_one_branch_and_a_read_only_in_the_other() {
    // The two branches are mutually exclusive — nothing races.
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  flag: Boolean = true\n  if flag do\n    logger.log(msg)\n  else\n    puts msg.text\n  end\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_read_after_an_if_whose_only_one_branch_sent() {
    // Decision log's conservative merge: the moved-set carried past the
    // whole `if` is the UNION of what every branch did, even though
    // only one branch could actually have executed on any given run.
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  flag: Boolean = true\n  if flag do\n    logger.log(msg)\n  end\n  puts msg.text\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("the conservative branch-merge rule must reject this, not full path sensitivity");
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn rejects_a_read_on_a_loops_own_last_statement_when_its_first_statement_sent() {
    // Loop conservatism: "textually after" has no fixed meaning inside
    // a loop body — a send anywhere in the body is checked against
    // every other use anywhere else in that SAME body.
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  var i: Int64 = 0\n  while i < 3 do\n    logger.log(msg)\n    puts msg.text\n    i = i + 1\n  end\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a send and a use in the same loop body must be rejected regardless of order");
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn rejects_a_read_after_a_loop_whose_body_sent() {
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  var i: Int64 = 0\n  while i < 3 do\n    logger.log(msg)\n    i = i + 1\n  end\n  puts msg.text\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err(
      "a send anywhere in a loop body must poison the local for all code after the loop",
    );
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn accepts_a_value_type_argument_reused_after_the_send() {
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  n: Int64 = 42\n  logger.ping(n)\n  puts n\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "an Int64 argument is a value type — copied at the send, reusable afterward"
    );
  }

  #[test]
  fn accepts_a_freshly_constructed_reference_argument() {
    let src = message_safety_program(
      "  logger: Logger = Logger.spawn()\n  logger.log(LogMessage.new(\"fresh\"))\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_field_read_passed_directly_as_a_message_argument() {
    let src = "class LogMessage\n  text: String\n\n  fn initialize(text: String): Void do\n    @text = text\n  end\n\n  fn text: String do\n    @text\n  end\nend\n\nactor Logger\n  fn log(msg: LogMessage): Void do\n    puts msg.text\n  end\nend\n\nclass Holder\n  msg: LogMessage\n\n  fn initialize(msg: LogMessage): Void do\n    @msg = msg\n  end\n\n  fn send_it(logger: Logger): Void do\n    logger.log(@msg)\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`@msg` can alias a binding that outlives the send — rejected outright");
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn rejects_an_index_read_passed_directly_as_a_message_argument() {
    let src = "class LogMessage\n  text: String\n\n  fn initialize(text: String): Void do\n    @text = text\n  end\nend\n\nactor Logger\n  fn log(msg: LogMessage): Void do\n  end\nend\n\nfn run: Void do\n  logger: Logger = Logger.spawn()\n  msgs: Array[LogMessage] = Array.new(1)\n  msgs[0] = LogMessage.new(\"x\")\n  logger.log(msgs[0])\nend\n\nrun()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`arr[i]` can alias a binding that outlives the send — rejected outright");
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn rejects_a_nested_call_result_passed_directly_as_a_message_argument() {
    let src = "class LogMessage\n  text: String\n\n  fn initialize(text: String): Void do\n    @text = text\n  end\nend\n\nfn build: LogMessage do\n  LogMessage.new(\"built\")\nend\n\nactor Logger\n  fn log(msg: LogMessage): Void do\n  end\nend\n\nfn run: Void do\n  logger: Logger = Logger.spawn()\n  logger.log(build())\nend\n\nrun()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err(
      "a nested call's return value can't be proven unaliased by this compiler — rejected outright",
    );
    assert!(errs.iter().any(|d| d.message.contains("message-safety")));
  }

  #[test]
  fn plan_56_regression_every_prior_actor_example_still_type_checks() {
    // No pre-plan-54 example uses actors at all (they can't — the
    // construct didn't exist); this is the narrowest real regression
    // check available: plan 54/55's own worked examples, unaffected by
    // this plan's new rule (neither sends a reference-typed local more
    // than once).
    let counter = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n\na.increment\na.increment\nb.increment\n\na.value\nb.value\n";
    let program = emerald_parser::parse(counter).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 57 (supervision trees).

  /// Shared `Worker`/`Logger` preamble — `after_supervise` supplies the
  /// `supervise do ...` block's own body PLUS its closing `end` PLUS
  /// anything that follows, since (unlike `message_safety_program`'s
  /// fixed-shape `run_body`) most scenarios here need real top-level
  /// statements after the block itself (e.g. a `.child(...)` query).
  fn supervisor_program(after_supervise: &str) -> String {
    format!(
      "actor Worker\n  count: Int64\n\n  fn initialize(seed: Int64): Void do\n    @count = seed\n  end\nend\n\nactor Logger\n  prefix: String\n\n  fn initialize(prefix: String): Void do\n    @prefix = prefix\n  end\nend\n\nsup: Supervisor = supervise do\n{after_supervise}"
    )
  }

  #[test]
  fn accepts_a_supervise_block_with_named_and_bare_spawns_and_a_named_child_query() {
    let src = supervisor_program(
      "  worker: Worker = Worker.spawn(0)\n  Logger.spawn(\"log\")\nend\n\nw: Worker = sup.child(:worker)\n",
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_non_spawn_statement_inside_a_supervise_block() {
    let src = supervisor_program("  puts \"x\"\nend\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a `supervise do ... end` body may only contain spawn statements");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("supervise do ... end")),
      "diagnostic must name the restricted body shape: {errs:?}"
    );
  }

  #[test]
  fn rejects_a_child_query_for_an_unnamed_bare_spawn() {
    // AC3 (`leaf-supervise-declaration`): a bare, unnamed `.spawn`
    // compiles, but is unreachable via `child(name)` — a real,
    // disclosed narrowing caught at compile time (the tracked-children
    // map sema builds never gets an entry for it), not a runtime crash.
    let src =
      supervisor_program("  Logger.spawn(\"log\")\nend\n\nl: Logger = sup.child(:logger)\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("an unnamed bare spawn must not be reachable via `.child(:logger)`");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("no tracked child named")),
      "{errs:?}"
    );
  }

  #[test]
  fn rejects_a_declared_type_that_does_not_match_the_spawned_class() {
    let src = supervisor_program("  worker: Logger = Worker.spawn(0)\nend\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err(
      "`worker: Logger = Worker.spawn(...)` — declared type disagrees with the spawned class",
    );
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("declared type must match")),
      "{errs:?}"
    );
  }

  // Plan 42 (enumerable stdlib).

  #[test]
  fn accepts_the_enumerable_worked_example() {
    let src = "is_even: Proc = do |x: Int64| x % 2 == 0 end\ndoubler: Proc = do |x: Int64| x * 2 end\n\narr: Array[Int64] = [1, 2, 3, 4, 5, 6]\nevens: Array[Int64] = arr.select(is_even)\ndoubled: Array[Int64] = evens.map(doubler)\ntotal: Int64 = doubled.sum()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_each_on_a_non_iterable_receiver() {
    // Real, disclosed correction found this session: `.each` on a
    // non-Array/Hash receiver is rejected by the ordinary, pre-
    // existing `Type::Class` fallthrough (`method call \`.each\` on
    // non-class type ...`), not a dedicated "non-Array/Hash" message —
    // this leaf's own dispatch is scoped to the receiver's real
    // inferred type (`infer_expr_type`'s shared `MethodCall` arm), the
    // same fix `.value`/`.sum` needed after an earlier, name-guarded
    // version broke `examples/classes.em`'s own real `Counter`/`Point`
    // methods.
    let src = "printer: Proc = do |x: Int64| puts x end\nn: Int64 = 5\nn.each(printer)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`.each` on a non-Array/Hash receiver must be rejected");
    assert!(errs.iter().any(|d| d.message.contains(".each")), "{errs:?}");
  }

  #[test]
  fn rejects_sum_on_a_non_numeric_element_type() {
    let src = "arr: Array[String] = [\"a\", \"b\"]\ntotal: String = arr.sum()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`.sum` on Array[String] must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("Int64 or Float64 element type")),
      "{errs:?}"
    );
  }

  #[test]
  fn rejects_a_select_proc_that_does_not_return_boolean() {
    let src = "not_bool: Proc = do |x: Int64| x end\narr: Array[Int64] = [1, 2, 3]\nevens: Array[Int64] = arr.select(not_bool)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`.select`'s Proc must return Boolean");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("must return Boolean")),
      "{errs:?}"
    );
  }

  #[test]
  fn rejects_a_proc_argument_with_the_wrong_arity() {
    let src = "no_args: Proc = do | | true end\narr: Array[Int64] = [1, 2, 3]\nevens: Array[Int64] = arr.select(no_args)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a Proc with the wrong parameter arity must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("parameter types")),
      "{errs:?}"
    );
  }

  #[test]
  fn hash_each_proc_parameter_resolves_to_a_real_pair_type() {
    let src = "printer: Proc = do |p: Pair[Int64, Int64]| puts p.key\n  puts p.value end\nh: Hash[Int64, Int64] = {1 => 10}\nh.each(printer)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 58 (generic types) — leaf-2/leaf-3 acceptance criteria.

  #[test]
  fn a_bound_less_generic_class_instantiates_and_typechecks() {
    let src = "class Stack[T]\n  top: T\nend\n\ns: Stack[Int64] = Stack.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn two_distinct_instantiations_of_the_same_generic_class_both_typecheck() {
    let src = "class Stack[T]\n  top: T\nend\n\na: Stack[Int64] = Stack.new()\nb: Stack[String] = Stack.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  const COMPARABLE_MONEY_SRC: &str = "\ninterface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  amount: Int64\n  fn compare_to(other: Money): Int64 do\n    @amount\n  end\nend\n";

  #[test]
  fn a_bounded_generic_class_instantiated_with_a_conforming_class_typechecks() {
    let src = format!(
      "{COMPARABLE_MONEY_SRC}\nclass Box[T: Comparable]\n  value: T\nend\n\nb: Box[Money] = Box.new()\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn a_bounded_generic_class_instantiated_with_a_non_conforming_class_is_rejected() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Plain\n  x: Int64\nend\n\nclass Box[T: Comparable]\n  value: T\nend\n\nb: Box[Plain] = Box.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`Plain` doesn't implement `Comparable` — must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("does not implement")),
      "{errs:?}"
    );
  }

  #[test]
  fn calling_a_method_on_an_unbounded_type_parameter_is_rejected_in_the_template_itself() {
    // Never instantiated anywhere — proves the template body checks once,
    // as a template, not lazily deferred to a call site that never comes.
    let src = "class Box[T]\n  fn show(x: T): Void do\n    x.compare_to(x)\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a method call on an unbounded type parameter must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("unbounded type parameter")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_self_referential_linked_list_style_generic_class_instantiates() {
    let src = "class Node[T]\n  value: T\n  succ: Node[T]\nend\n\nn: Node[Int64] = Node.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn an_unboundedly_recursive_generic_field_is_rejected_with_a_depth_diagnostic() {
    let src =
      "class Box[T]\n  value: T\n  wrapped: Box[Box[T]]\nend\n\nb: Box[Int64] = Box.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("Box[T]'s own Box[Box[T]] field must be rejected as unbounded recursion");
    assert!(
      errs.iter().any(|d| d.message.contains("nesting depth")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_two_class_generic_cycle_is_rejected_with_a_depth_diagnostic() {
    let src = "class A[T]\n  b: B[T]\nend\n\nclass B[T]\n  a: A[T]\nend\n\nx: A[Int64] = A.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("a two-class generic cycle must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("nesting depth")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_generic_class_instantiated_with_an_undefined_type_argument_is_rejected() {
    let src = "class Stack[T]\n  top: T\nend\n\ns: Stack[NotAClass] = Stack.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("`Stack[NotAClass]` must be rejected — `NotAClass` is undefined");
    assert!(
      errs.iter().any(|d| d.message.contains("unknown type")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_bare_generic_new_with_no_expected_type_context_is_rejected() {
    // AC7: `Stack.new()` with no adjacent Let annotation to provide the
    // concrete type argument resolves against the bare, unmangled
    // "Stack" — never registered in `classes` (only its instantiations
    // are) — so this fails via the ordinary `Expr::New` "undefined
    // class" diagnostic, with no special-casing needed for this shape.
    let src = "class Stack[T]\n  top: T\nend\n\nStack.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a bare `Stack.new()` with no context must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("undefined class")),
      "{errs:?}"
    );
  }

  #[test]
  fn ordinary_classes_still_typecheck_fine_alongside_a_generic_class() {
    let src = "class Point\n  x: Int64\n  y: Int64\nend\n\nclass Stack[T]\n  top: T\nend\n\np: Point = Point.new()\ns: Stack[Int64] = Stack.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 59 (C FFI).

  #[test]
  fn extern_c_fns_register_and_type_check_calls_against_their_declared_signature() {
    let src = "unsafe extern \"C\" {\n  fn llabs(x: Int64): Int64\n  fn strlen(s: String): Int64\n}\n\nx: Int64 = llabs(-42)\nn: Int64 = strlen(\"hello\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn extern_fn_with_an_unsupported_parameter_type_is_rejected_naming_the_type() {
    let src = "unsafe extern \"C\" {\n  fn f(x: Boolean): Int64\n}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Boolean is not a marshalable extern type");
    assert!(
      errs.iter().any(|d| d.message.contains("Boolean")),
      "{errs:?}"
    );
  }

  #[test]
  fn extern_fn_with_an_unsupported_class_return_type_is_rejected() {
    let src = "class Foo\nend\n\nunsafe extern \"C\" {\n  fn f(): Foo\n}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("a user Class is not a marshalable extern type");
    assert!(errs.iter().any(|d| d.message.contains("Foo")), "{errs:?}");
  }

  #[test]
  fn extern_fn_with_an_unsupported_array_parameter_type_is_rejected() {
    let src = "unsafe extern \"C\" {\n  fn f(x: Array[Int64]): Int64\n}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Array[_] is not a marshalable extern type");
    assert!(
      errs.iter().any(|d| d.message.contains("Array[Int64]")),
      "{errs:?}"
    );
  }

  #[test]
  fn an_extern_block_with_a_non_c_abi_is_rejected() {
    let src = "unsafe extern \"C++\" {\n  fn f(): Int64\n}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("only \"C\" is a supported extern ABI");
    assert!(errs.iter().any(|d| d.message.contains("C++")), "{errs:?}");
  }

  #[test]
  fn string_to_cstring_type_checks_to_cstring() {
    let src = "s: String = \"hello\"\nc: CString = s.to_cstring()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn string_from_cstring_type_checks_to_option_string_and_rejects_a_direct_method_call() {
    let src = "unsafe extern \"C\" {\n  fn f(): CString\n}\n\nr: Option[String] = String.from_cstring(f())\nputs r.length\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a direct method call on the un-narrowed Option[String] result must be rejected");
    assert!(!errs.is_empty(), "{errs:?}");
  }

  #[test]
  fn string_from_cstring_called_on_a_non_cstring_argument_is_a_real_diagnostic() {
    let src = "puts String.from_cstring(42)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("String.from_cstring on a bare Int64 must be rejected");
    assert!(!errs.is_empty(), "{errs:?}");
  }

  // Plan 60 (distributed, location-transparent actors).

  #[test]
  fn register_on_an_actor_receiver_type_checks() {
    let src = "actor Counter\n  count: Int64\nend\n\nc: Counter = Counter.spawn()\nc.register(\"counter1\", 9000)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn register_on_a_non_actor_receiver_is_rejected() {
    let src = "class Widget\nend\n\nw: Widget = Widget.new()\nw.register(\"widget1\", 9001)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`.register` on a non-actor class must be rejected");
    assert!(!errs.is_empty(), "{errs:?}");
  }

  #[test]
  fn remote_on_a_non_actor_class_is_rejected() {
    let src = "class Widget\nend\n\nw: Widget = Widget.remote(\"127.0.0.1:9000\", \"widget1\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`.remote` only resolves actors");
    assert!(
      errs.iter().any(|d| d.message.contains("cannot `.remote`")),
      "{errs:?}"
    );
  }

  // Plan 56's own message-safety pass only ever runs over a function/
  // method body (`check_function_body`/`check_method_body`'s own call
  // sites) — a bare top-level send is a real, pre-existing, disclosed
  // gap (see plan 56's own Decision log), not something plan 60
  // changes. Every wire-safety test below wraps its send inside a
  // `def main()` for exactly this reason, matching plan 56's own
  // worked examples' established convention.

  #[test]
  fn an_array_message_argument_is_rejected_as_not_wire_safe() {
    let src = "actor Logger\n  fn log(items: Array[Int64]): Void do\n  end\nend\n\nfn main(): Void do\n  l: Logger = Logger.spawn()\n  arr: Array[Int64] = [1, 2, 3]\n  l.log(arr)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Array[_] is not wire-safe");
    assert!(
      errs.iter().any(|d| d.message.contains("not wire-safe")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_proc_message_argument_is_rejected_as_not_wire_safe() {
    let src = "actor Logger\n  fn run(f: Proc): Void do\n  end\nend\n\nfn main(): Void do\n  l: Logger = Logger.spawn()\n  cb: Proc = do | |  end\n  l.run(cb)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Proc is not wire-safe");
    assert!(
      errs.iter().any(|d| d.message.contains("not wire-safe")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_second_actor_reference_message_argument_is_rejected_as_not_wire_safe() {
    let src = "actor Pinger\n  fn notify(other: Pinger): Void do\n  end\nend\n\nfn main(): Void do\n  a: Pinger = Pinger.spawn()\n  b: Pinger = Pinger.spawn()\n  a.notify(b)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("sending an actor reference is not wire-safe");
    assert!(
      errs.iter().any(|d| d.message.contains("not wire-safe")),
      "{errs:?}"
    );
  }

  #[test]
  fn plan_56_liveness_still_fires_unchanged_on_an_actor_send() {
    let src = "actor Receiver\n  fn take(p: Payload): Void do\n  end\nend\n\nclass Payload\n  data: String\nend\n\nfn main(): Void do\n  r: Receiver = Receiver.spawn()\n  p: Payload = Payload.new()\n  r.take(p)\n  r.take(p)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("reusing `p` after it was sent must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("cannot be used again")),
      "{errs:?}"
    );
  }

  // Plan 62 (design-by-contract).

  const DIVIDE_SRC: &str = "fn divide(a: Int64, b: Int64): Int64\n  requires b != 0\n  ensures result * b <= a do\n  return a / b\nend\n";

  #[test]
  fn divide_worked_example_type_checks_ok() {
    let src = format!("{DIVIDE_SRC}\ny: Int64 = 10\nz: Int64 = 2\nputs divide(y, z)\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn a_non_boolean_requires_clause_is_rejected_naming_the_clause_and_type() {
    let src = "fn bad(a: Int64): Int64\n  requires a do\n  return a\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("a non-Boolean requires must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("requires") && d.message.contains("Int64")),
      "{errs:?}"
    );
  }

  #[test]
  fn an_ensures_clause_referencing_an_undeclared_identifier_is_rejected() {
    let src = "fn bad2(a: Int64): Int64\n  ensures unknown_name do\n  return a\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("an undeclared identifier must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("unknown_name")),
      "{errs:?}"
    );
  }

  #[test]
  fn ensures_result_resolves_to_the_declared_return_type() {
    let src = "fn ok(a: Int64): Int64\n  ensures result >= 0 do\n  return a\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn requires_on_a_class_method_via_a_hand_constructed_ast_is_rejected() {
    let mut program =
      emerald_parser::parse("class Point\n  x: Int64\nend\n").expect("should parse");
    let Item::Class(c) = &mut program.items[0] else {
      panic!("expected a class");
    };
    c.methods.push(Function {
      name: "get_x".to_string(),
      params: Vec::new(),
      return_type: TypeExpr::Named("Int64".to_string()),
      body: vec![Spanned::synthetic(Stmt::Return(Some(Spanned::synthetic(
        Expr::Int(0),
      ))))],
      block_param: None,
      splat_param: None,
      type_params: Vec::new(),
      is_comptime: false,
      requires: vec![Contract {
        expr: Spanned::synthetic(Expr::Bool(true)),
        text: "true".to_string(),
        line: 0,
      }],
      ensures: Vec::new(),
      is_pure: false,
    });
    let errs = check_program(&program).expect_err("contracts on a method must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("not supported on methods yet")),
      "{errs:?}"
    );
  }

  #[test]
  fn requires_on_a_generic_function_is_rejected() {
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nfn identity[T: Comparable](x: T): T\n  requires true do\n  return x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("contracts on a generic function must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("not supported on generic functions yet")),
      "{errs:?}"
    );
  }

  #[test]
  fn a_literal_zero_divisor_is_rejected_at_compile_time() {
    let src = format!("{DIVIDE_SRC}\nputs divide(10, 0)\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err("divide(10, 0) must be rejected at compile time");
    assert!(
      errs.iter().any(|d| {
        d.message
          .contains("contract violation provable at compile time")
          && d.message.contains("b != 0")
          && d.message.contains("b = 0")
      }),
      "{errs:?}"
    );
  }

  #[test]
  fn let_bound_locals_at_a_call_site_are_not_statically_checked() {
    let src = format!("{DIVIDE_SRC}\ny: Int64 = 10\nz: Int64 = 0\nputs divide(y, z)\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "a Let-bound local must never be statically substituted, even if it happens to be zero"
    );
  }

  #[test]
  fn a_computed_non_literal_argument_is_not_statically_checked() {
    let src = format!("{DIVIDE_SRC}\nx: Int64 = 3\nw: Int64 = x - 3\nputs divide(20, w)\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "a computed expression argument must never be statically substituted"
    );
  }

  #[test]
  fn a_clause_with_one_literal_and_one_non_literal_argument_is_skipped_entirely() {
    let src = "fn f(a: Int64, b: Int64): Int64\n  requires a + b > 0 do\n  return a + b\nend\n\ny: Int64 = 10\nputs f(5, y)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "a clause referencing any non-literal-bound identifier must be skipped, not partially folded"
    );
  }

  // Plan 63's `leaf-sema-purity-check`.

  #[test]
  fn a_self_recursive_pure_function_is_accepted() {
    let src = "pure fn fib(n: Int64): Int64 do\n  if n < 2 do\n    n\n  else\n    fib(n - 1) + fib(n - 2)\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "self-recursion is the size-one cyclic-component case, accepted under the cycle rule"
    );
  }

  #[test]
  fn genuine_mutual_recursion_both_pure_claimed_is_accepted_together() {
    let src = "pure fn is_even(n: Int64): Boolean do\n  if n == 0 do\n    true\n  else\n    is_odd(n - 1)\n  end\nend\n\npure fn is_odd(n: Int64): Boolean do\n  if n == 0 do\n    false\n  else\n    is_even(n - 1)\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "a genuine mutual-recursion pair, both `pure`-claimed, must be accepted as one component"
    );
  }

  #[test]
  fn a_mutual_recursion_pair_where_one_member_calls_puts_is_rejected_as_a_whole_component() {
    let src = "pure fn is_even(n: Int64): Boolean do\n  if n == 0 do\n    true\n  else\n    is_odd(n - 1)\n  end\nend\n\npure fn is_odd(n: Int64): Boolean do\n  puts n\n  if n == 0 do\n    false\n  else\n    is_even(n - 1)\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a component with one I/O-performing member must be rejected");
    assert_eq!(
      errs.len(),
      2,
      "both mutually-recursive members must get their own diagnostic"
    );
    assert!(
      errs.iter().any(|d| d.message.contains("puts")),
      "the member that actually calls `puts` must have its own diagnostic naming it: {errs:?}"
    );
  }

  #[test]
  fn a_pure_function_calling_puts_is_rejected_naming_puts() {
    let src = "pure fn bad(x: Int64): Int64 do\n  puts x\n  return x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a `pure` function performing I/O must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("puts") && d.span != (0, 0)),
      "the diagnostic must name `puts` and carry a real span: {errs:?}"
    );
  }

  #[test]
  fn a_pure_function_sending_to_an_actor_is_rejected_naming_the_send() {
    let src = "actor Worker\n  fn run(n: Int64): Void do\n    puts n\n  end\nend\n\npure fn bad2(w: Worker): Void do\n  w.run(5)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a `pure` function sending a message must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("actor") || d.message.contains("message")),
      "the diagnostic must name the cross-actor send: {errs:?}"
    );
  }

  #[test]
  fn a_pure_function_setting_a_field_is_rejected() {
    let src =
      "class Counter\n  n: Int64\n\n  pure fn bump(): Void do\n    @n = @n + 1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a `pure` method mutating a field must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("field")),
      "the diagnostic must name the field mutation: {errs:?}"
    );
  }

  #[test]
  fn a_pure_function_setting_an_index_is_rejected() {
    let src = "pure fn bad(arr: Array[Int64]): Void do\n  arr[0] = 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a `pure` function mutating an index must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("index")),
      "the diagnostic must name the index mutation: {errs:?}"
    );
  }

  #[test]
  fn a_pure_function_calling_an_ordinary_function_is_rejected_naming_the_callee() {
    let src = "fn helper(x: Int64): Int64 do\n  return x + 1\nend\n\npure fn bad(x: Int64): Int64 do\n  return helper(x)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a `pure` function calling a non-`pure` function must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("helper")),
      "the diagnostic must name the non-`pure` callee: {errs:?}"
    );
  }

  #[test]
  fn pure_declared_on_an_actor_method_is_rejected_at_registration() {
    let src = "actor Worker\n  pure fn run(n: Int64): Void do\n    puts n\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`pure` on an actor method must be rejected");
    assert!(
      errs.iter().any(|d| d.message.contains("actor method")),
      "the diagnostic must name the actor-method restriction: {errs:?}"
    );
  }

  #[test]
  fn an_ordinary_program_using_pure_nowhere_typechecks_identically() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  return a + b\nend\n\nputs add(2, 3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(
      check_program(&program),
      Ok(()),
      "`check_purity` must be a strict no-op over a program that never writes the word `pure`"
    );
  }

  // Plan 70 (enumerable stdlib completion): `map`/`reduce`/
  // `each_with_index` on `Hash[K,V]`, and the new predicate form of
  // `.count`.

  #[test]
  fn hash_map_over_pairs_type_checks_to_an_array_of_the_procs_own_return_type() {
    let src = "double_value: Proc = do |p: Pair[Int64, Int64]| p.value * 2 end\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20}\nvalues: Array[Int64] = h.map(double_value)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn hash_reduce_over_pairs_folds_to_the_initial_values_own_type() {
    let src = "sum_values: Proc = do |acc: Int64, p: Pair[Int64, Int64]| acc + p.value end\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20}\ntotal: Int64 = h.reduce(0, sum_values)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn hash_each_with_index_accepts_a_pair_then_index_proc() {
    let src = "visit: Proc = do |p: Pair[Int64, Int64], i: Int64| puts i end\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20}\nh.each_with_index(visit)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn hash_select_stays_array_only_since_array_of_pair_cannot_be_named() {
    // Real, disclosed narrowing (`check_enumerable_call`'s own doc
    // comment): `Array[Pair[K,V]]` — `.select`'s only sensible result
    // type on a Hash — can never be written in this language's
    // concrete syntax, so `.select`/`.filter` stay Array-only.
    let src = "is_big: Proc = do |p: Pair[Int64, Int64]| p.value > 15 end\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20}\nh.select(is_big)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("`.select` on a Hash[K,V] receiver must be rejected");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("is only supported on Array[T]")),
      "{errs:?}"
    );
  }

  #[test]
  fn array_count_with_a_predicate_proc_type_checks_to_int64() {
    let src = "is_big: Proc = do |x: Int64| x > 2 end\narr: Array[Int64] = [1, 2, 3, 4]\nn: Int64 = arr.count(is_big)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_count_predicate_that_does_not_return_boolean() {
    let src = "not_bool: Proc = do |x: Int64| x end\narr: Array[Int64] = [1, 2, 3]\nn: Int64 = arr.count(not_bool)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`.count`'s predicate Proc must return Boolean");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("predicate Proc must return Boolean")),
      "{errs:?}"
    );
  }
}
