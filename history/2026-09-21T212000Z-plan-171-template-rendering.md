2026-09-21T21:20:00Z

---
name: Template Rendering — `tera` Wrapped as `Template`
overview: "A `Template` module (`Template.render(source, context): String`) wrapping `tera` 2.4's single-string rendering path (`Tera::default()` + `add_raw_template` + `render`, never the crate's directory-glob loader), with two new opaque builder handles — `TemplateContext` (scalar key/value pairs) and `TemplateArray` (a `{% for %}`-iterable list) — covering Jinja2/Django-style `{{ variable }}` interpolation, `{% if %}`/`{% endif %}` conditionals, and `{% for %}`/`{% endfor %}` loops; `tera` is chosen over `minijinja` specifically because `tera::Context` is built directly from `serde_json`-`Serialize`-able values, the same representation shape plan 118's dynamic-value type is expected to land in, so a future `TemplateContext.set_value` overload composes with plan 118 with no separate value-bridging layer to write."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-emerald-rt-tera-integration
    content: "Add `tera = { version = \"2.4\", default-features = false }` to `crates/emerald-rt/Cargo.toml`; confirm (leaf-level verification task, not asserted here) whether `default-features = false` still supports `Tera::default()`/`add_raw_template`/`render` without pulling `globset`/`walkdir` (both real, optional, glob-directory-loading-only dependencies per `tera` 2.4.0's own `Cargo.toml`, verified this session) — this project's v1 API never calls `Tera::new(glob)`/`load_from_glob`, so those two crates have no runtime role to justify linking them."
    status: pending
  - id: leaf-template-context-handle
    content: "A new opaque `TemplateContext` handle wrapping a real `tera::Context`: `emerald_rt_template_context_new() -> *mut TemplateContext`, `emerald_rt_template_context_set_string/int/float/bool(handle, key: *const c_char, value: ...) -> i64` (each a thin `Context::insert(key, &value)` call), and `emerald_rt_template_context_set_array(handle, key: *const c_char, arr: *mut TemplateArray) -> i64` (consumes the `TemplateArray` handle, inserting its accumulated `Vec<String>` as one context entry)."
    status: pending
  - id: leaf-template-array-handle
    content: "A second new opaque `TemplateArray` handle wrapping a plain `Vec<String>`: `emerald_rt_template_array_new() -> *mut TemplateArray`, `emerald_rt_template_array_push_string(handle, value: *const c_char) -> i64` — string-only in v1 for the same reason plan 168's `LogFields` is string-only (see Decision log)."
    status: pending
  - id: leaf-render
    content: "`emerald_rt_template_render(source: *const c_char, context: *mut TemplateContext) -> *mut c_char`: builds one throwaway `Tera::default()`, `add_raw_template(\"__emerald_template.html\", source)` (the fixed `.html`-suffixed internal name, deliberately chosen so `tera`'s real suffix-based autoescape rule — verified this session directly against `tera` 2.4.0's own `autoescape_on` doc comment: \"By default, autoescaping is performed on `.html`, `.htm` and `.xml` template files\" — applies by default), `render(\"__emerald_template.html\", &context)`, and returns the rendered `String`; consumes and frees the `TemplateContext` handle. A `tera::Error` (parse failure or missing/undefined variable) is a disclosed runtime abort in v1, matching plan 45's own `File.read`/`File.write` error precedent, not wired into `raise`/`rescue`."
    status: pending
  - id: leaf-sema-codegen-dispatch
    content: "`Template.render(source: String, context: TemplateContext): String`; `TemplateContext.new(): TemplateContext` and `.set_string`/`.set_int`/`.set_float`/`.set_bool`/`.set_array` as ordinary methods on the new opaque `TemplateContext` type; `TemplateArray.new(): TemplateArray` and `.push_string` likewise — the same intrinsic-dispatch shape plan 45/168/169/170 already established, extended here to two *user-instantiable* opaque types (`TemplateContext.new()`/`TemplateArray.new()` are static factory calls on a compiler-known non-`ModuleDef` name, not just namespaced functions like `File`/`Log`/`Metrics`/`Trace`)."
    status: pending
  - id: leaf-example-and-tests
    content: "`examples/template_rendering_proof.em` (Concrete Proof below); `#[test]`s in `emerald-rt` asserting `emerald_rt_template_render`'s output for a fixed source/context matches an exact expected HTML string, and that a template referencing an undefined context variable aborts rather than silently rendering an empty string."
    status: pending
isProject: false
---

# Plan 171 — Template Rendering

This is the fourth of eight sibling plans (168-175) in the 91-191
batch. Unlike the observability trio before it (168-170), this plan
adds nothing operators need to watch a running program with — it adds
a capability programs themselves use, generating HTML (or any other
text format) from a template string and a data context, the same job
Jinja2/ERB/Handlebars do in their respective ecosystems.

## Concrete proof this plan targets

```ruby
ctx: TemplateContext = TemplateContext.new()
ctx.set_string("title", "Welcome")
ctx.set_bool("logged_in", true)

items: TemplateArray = TemplateArray.new()
items.push_string("apples")
items.push_string("bananas")
items.push_string("cherries")
ctx.set_array("items", items)

source: String = "<h1>{{ title }}</h1>\n{% if logged_in %}<p>Hello!</p>{% endif %}\n<ul>\n{% for item in items %}<li>{{ item }}</li>\n{% endfor %}</ul>\n"

html: String = Template.render(source, ctx)
puts html
```

Expected output:

```
<h1>Welcome</h1>
<p>Hello!</p>
<ul>
<li>apples</li>
<li>bananas</li>
<li>cherries</li>
</ul>
```

A real proof of all three syntax basics the brief asks for in one pass:
`{{ title }}` interpolation, `{% if logged_in %}` conditional, and
`{% for item in items %}` iteration over a genuinely dynamic,
runtime-built list — not a compile-time-known literal.

## Decision log

- **`tera` vs `minijinja`: a real comparative call, decided by which
  one composes with plan 118, not by general popularity.** Both are
  actively maintained, verified today: `tera` `2.4.0`, published 11
  September 2026, owned by `Keats`, MIT; `minijinja` `2.24.0`,
  published 12 August 2026, owned by `mitsuhiko` (Armin Ronacher —
  Jinja2's original author, also Flask's), Apache-2.0. `minijinja`'s
  real, genuine advantages: a single, unusually reputable maintainer,
  and a leaner default dependency set (no `globset`/`walkdir`-shaped
  directory-loading machinery to opt out of at all, since it was never
  built in). The deciding factor is architectural, not maintainer
  reputation: `tera::Context` is, structurally, a `serde_json::Map`
  under a thin wrapper — `Context::insert(key, value)` takes anything
  `serde::Serialize`, and `tera::Value` is a direct re-export of
  `serde_json::Value` — while `minijinja::Value` is that crate's own,
  separate dynamic-value type, requiring its own conversion path from
  whatever plan 118 lands on. Since plan 118's whole job is defining
  Emerald's dynamic-value representation, and the most natural Rust
  shape for "a JSON-like dynamic value" is exactly `serde_json::Value`
  or something isomorphic to it, `tera` gives this plan (and a future
  plan-118-aware revision of it) one fewer value-model translation
  layer to maintain than `minijinja` would. This plan is not itself
  blocked on plan 118 (see the scalar/array-only scope below), but the
  choice is made with that future integration point already accounted
  for, not merely popularity.
- **v1 uses `Tera::default()` + `add_raw_template` + `render`, never
  `Tera::new(glob)`/`load_from_glob` — verified as the real, documented
  no-filesystem rendering path, not inferred.** `tera` 2.4.0's own
  `Tera` doc comment (checked this session, directly against
  `docs.rs`) states its own worked example this exact way: `let mut
  tera = Tera::default(); tera.add_raw_template("hello", "Hello, {{
  name }}!").unwrap(); ...tera.render("hello", &context).unwrap()`.
  Loading a directory of templates by glob pattern (`tera`'s other,
  more commonly documented entry point) is a different use case this
  plan does not need — Emerald source hands `Template.render` a
  complete template `String` it already obtained however it likes
  (a literal, or `File.read` per plan 144), matching this project's
  existing "the stdlib module does one narrow job, the caller supplies
  the string" posture from `File.write`/`File.read` (plan 45) rather
  than growing its own file-discovery logic.
- **The internal template name is a fixed, deliberately `.html`-suffixed
  string, chosen specifically to turn on `tera`'s real default
  autoescaping — a precise, checked behavior, not a guess.** `tera`
  2.4.0's `autoescape_on` doc comment states plainly: "By default,
  autoescaping is performed on `.html`, `.htm` and `.xml` template
  files" — the rule keys off the *name* a template is registered
  under, not its content or any explicit flag on `render`. Every
  `Template.render` call registers its source under the fixed internal
  name `"__emerald_template.html"`, so HTML-escaping of interpolated
  values (`{{ title }}` escaping `<`/`>`/`&`/quotes) is on by default —
  the safe default for the overwhelmingly common "render HTML from
  user-influenced data" case. There is no `Template.render_unescaped`
  in this plan; declining autoescaping entirely (e.g., for generating
  plain text or non-HTML markup where escaping is actively wrong) is a
  real, disclosed gap left for a follow-up that threads a second
  internal name suffix (or `tera`'s own `autoescape_on` override)
  through the API, not built here.
- **`TemplateContext`/`TemplateArray` are scalar/string-only in v1,
  for the identical reason plan 168's `LogFields` is: `Hash[K,V]`'s
  and `Array[T]`'s runtime enumeration mechanisms were not verified
  this session against real, current `emerald-sema`/`emerald-codegen`
  source, and this plan declines to build a marshaling path on an
  unverified foundation.** Two new opaque builder handles — accumulate
  scalar entries or string list entries one FFI call at a time — sidestep
  the question exactly as `LogFields` does, and are the concrete
  mechanism that makes the worked proof's `{% for item in items %}`
  loop real rather than aspirational: `items` is a genuinely
  runtime-constructed `Vec<String>` on the Rust side, iterated by
  `tera` itself, with no dependence on Emerald's own `Array[T]`
  iteration machinery (which plan 45's own Decision log already
  documents as absent for anything but a literal array — this plan
  does not need it, since the list lives entirely inside the
  `TemplateArray` handle until `tera` consumes it).
- **Nested/structured context values (an array of objects, a map
  inside a map) are out of scope in v1 — the two handles above are
  deliberately flat.** Real Jinja2/Tera templates routinely do `{% for
  user in users %}{{ user.name }}{% endfor %}` over a list of records,
  which this plan's `TemplateArray` (string elements only) cannot
  express. This is the direct, disclosed cost of declining plan 118's
  dependency rather than an oversight — a `TemplateContext.set_value`
  overload taking plan 118's real dynamic-value type, once it exists,
  is the natural, additive way to reach nested contexts, converting
  that value into a real `serde_json::Value` (a mechanical, well-typed
  conversion given `tera::Value`'s re-export relationship to
  `serde_json`, per the comparative-call reasoning above) and calling
  `Context::insert` with it directly — not a redesign of what ships
  here.
- **A `tera::Error` (template parse failure, or `{{ undefined_var }}`
  referencing a context key that was never set) is a disclosed runtime
  abort in v1, matching `File.read`/`File.write`'s existing precedent
  exactly, not routed through `raise`/`rescue`.** Plan 45's own
  Decision log states this choice for File I/O plainly: a real,
  controlled failure path (`fprintf(stderr, ...); exit(1)`-shaped, not
  silent UB), disclosed rather than hidden, deferred to a future
  errno-or-`tera::ErrorKind`-to-exception-class mapping this plan does
  not attempt. `Template.render` inherits that same posture for the
  same reason: a recoverable, `rescue`-able template error needs a
  real exception-class hierarchy for template failures specifically,
  which is separate, larger design work this plan's one intrinsic does
  not need in order to prove template rendering works at all.
- **`TemplateContext.new()`/`TemplateArray.new()` are the first
  *user-instantiable* opaque handles in this batch's stdlib surface so
  far — a real, if small, precedent-setting shape worth naming.**
  Plan 168's `LogFields.new()` already did this once, but every other
  new type in plans 168-170 (`TraceSpan`, `Metrics`' internal state)
  is produced only by a factory *method* on a fixed module name
  (`Trace.span_start`), never a bare `TypeName.new()` static
  constructor call on the type itself. `TemplateContext`/
  `TemplateArray` establish `TypeName.new()` as a second-class but
  real dispatch shape alongside `Name.method(args)` — both routed
  through the identical `matches!(recv.as_ref(), Expr::Ident(n) if n
  == "TemplateContext")`-style sema arm, since neither type is ever
  declared via a source `ModuleDef`/`ClassDef` either.
- **FFI/ABI notes.** Plan 92's general convention (`catch_unwind`,
  `i64`/opaque-pointer returns) governs every function here; its
  binary-safe `(ptr, len)` buffer convention is not needed — template
  sources, rendered output, and every context/array entry are plain
  null-terminated `String`s (plan 59), and `TemplateContext`/
  `TemplateArray` are opaque pointers exactly like plan 168's
  `LogFields` and plan 170's `TraceSpan`.
- **Out of scope.** Template inheritance (`{% extends %}`/`{% block
  %}`), includes (`{% include %}`), custom filters/functions/testers
  registered from Emerald source, multi-template `Tera` instances with
  a persistent cache (every `Template.render` call builds and discards
  its own throwaway `Tera` instance — real, disclosed per-call
  overhead, acceptable for v1's non-hot-path use cases), custom
  delimiters, nested/structured context values (see above — plan
  118's job), and any `raise`/`rescue`-integrated template error type.
