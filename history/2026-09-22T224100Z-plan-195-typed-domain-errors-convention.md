2026-09-22T22:41:00Z

---
name: "Typed Domain Errors — Result[T, <Domain>Error] Convention, Replacing Bare Result[T, String]"
overview: "inception-3's own §4.4 names a real, batch-wide gap: nearly every plan in 91-191 that can fail returns `Result[T, String]` — a caller can never branch on error *kind*, despite the language already having real enums (plan 52) and `Result[T,E]`/`?`-propagation (plan 53) to do this properly. This plan is a cross-cutting policy document in plan 95's own shape (a convention and a checklist, not a crate-wrap): it defines the `Result[T, <Domain>Error]` convention precisely, extends plan 92's FFI/ABI additively with a tagged-error export so a native error's real kind survives the crossing into Emerald rather than being flattened to a string first, and retrofits it into two already-shipped plans (118 JSON, 122 regex) as real, concrete worked examples — not just asserted in the abstract."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-define-the-convention
    content: "Write the convention itself, precisely, as this plan's own canonical definition every future domain-plan author checks against (mirroring plan 95's own role for crate-vetting): each domain that owns a `Result[T, String]`-shaped fallible operation defines a small, real enum named `<Domain>Error` (`JsonError`, `RegexError`, `GzipError`, ...) local to that domain — never one shared cross-domain giant error enum, since Emerald's domains (JSON, TOML, CSV, regex, gzip, ...) have no shared error taxonomy worth forcing into a single ADT, and a shared enum would mean every domain's variant list grows every time an unrelated domain's error surface changes. Every `<Domain>Error` enum carries exactly one non-domain-specific escape-hatch variant, `Other(String)`, for a native error this domain's author has not (yet) enumerated a real variant for — required, not optional, because no domain plan can front-load an exhaustive enumeration of every failure a wrapped crate can produce (see Decision log); a domain's own variant list is expected to grow over time as real `Other(...)` cases are observed and promoted to named variants, the same 'grows from real evidence, not speculation' discipline plan 95 already applies to `DEPENDENCIES.md`'s own rows."
    status: done
  - id: leaf-ffi-tagged-error-export
    content: "Extend plan 92's FFI/ABI convention additively (plan 92's own file is not edited — immutable per this project's history convention; this is a later plan amending it, the same relationship plan 93/94 already have to plan 92): add `emerald_rt_result_err_tagged(tag: i32, msg: *const c_char) -> *mut c_void` to `crates/emerald-rt`'s ABI surface, alongside plan 92's existing `emerald_rt_result_err(msg: *const c_char)` (kept unchanged, still used verbatim by every plan that has not adopted this convention — no retrofit of already-shipped native code is forced by this leaf). `tag` is a small, per-domain `i32` discriminant a domain's own Rust module assigns to its own `<Domain>Error` variants (e.g. `JsonError::Syntax = 0`, `JsonError::UnexpectedEnd = 1`, `JsonError::Other = 2`) — owned and interpreted entirely by that domain's own `emerald-sema`/`emerald-codegen` dispatch arm, never a global cross-domain tag registry, matching `leaf-define-the-convention`'s own per-domain-enum decision. `msg` stays populated on every variant, including named ones, not just `Other` — a real, human-readable message costs nothing extra and every domain's own error `Display`/`.to_s` still wants text, not just a bare variant name."
    status: done
  - id: leaf-retrofit-json-worked-example
    content: "Retrofit plan 118 (already shipped) as this plan's first real worked example, per the Concrete Proof below: `Json.parse(s: String): Result[JsonValue, String]` becomes `Json.parse(s: String): Result[JsonValue, JsonError]`, `JsonError = Syntax(String) | UnexpectedEnd | Other(String)`, the Rust side classifying via `serde_json::Error`'s own real `.classify(): serde_json::error::Category` method (verified against the actually-vendored `serde_json` version at execute time, not assumed from this plan's text — `Category::Syntax`/`Category::Eof`/`Category::Io`/`Category::Data` is `serde_json`'s own real, existing taxonomy as of the versions plan 118 itself cites; this leaf maps it onto `JsonError` rather than inventing a parallel one) before calling `emerald_rt_result_err_tagged`. A caller migration note: any already-written `.em` code matching `Err(msg) do ... end` against plan 118's original `String`-shaped error (plan 118's own Concrete Proof example) breaks under this retrofit — call out explicitly as a real, disclosed breaking change to plan 118's public surface, not hidden inside 'just an internal representation change,' and update plan 118's own example file (`examples/*.em` per its checked-table entry) to match rather than leaving it silently stale."
    status: done
  - id: leaf-retrofit-regex-worked-example
    content: "Retrofit plan 122 (already shipped) as the second worked example: `Regex.compile(pattern: String): Result[Regex, String]` becomes `Regex.compile(pattern: String): Result[Regex, RegexError]`, `RegexError = Syntax(String) | Other(String)` — the `regex` crate's own real error type must be checked directly against the exact version pinned in `crates/emerald-rt/Cargo.toml` at execute time (not assumed here) for how finely its variants can actually be distinguished; if it exposes only a syntax-error path in practice, a two-variant enum (`Syntax`/`Other`) is a legitimate, honest v1 scope rather than inventing variants the underlying crate cannot actually produce. Same breaking-change disclosure and example-file update obligation as the JSON retrofit."
    status: done
  - id: leaf-checklist-for-future-plans
    content: "Add a short, numbered checklist to this plan's own body (not a separate file) that any not-yet-implemented domain plan (96-191 minus 118/122, all of which currently specify `Result[T, String]` in their own Concrete Proof sections) should apply at EXECUTE time rather than retrofitting later: (1) define `<Domain>Error` with `Other(String)` at minimum; (2) call `emerald_rt_result_err_tagged`, not the untagged `emerald_rt_result_err`, from new native code; (3) update the plan's own Concrete Proof `.em` example to show the typed `Err(...)` match, not a bare string match; (4) if the plan's own already-authored text currently shows `Result[T, String]`, treat that text as superseded by this convention going forward without editing the original file (this project's own immutable-history convention — a later plan, or this plan's own retrofit precedent, is where the correction lives, not an edit to the original). This item is advisory to plans not yet executed, not a mandate this plan can itself enforce — flagged honestly as such in the Decision log."
    status: done
isProject: false
---

# Plan 195 — Typed Domain Errors Convention

inception-3 (`history/2026-09-21T195000Z-inception-3-stdlib-supremacy.md`
§4.4) names this the cheapest of its four real gaps to fix, and the one
requiring no new language mechanism at all — Emerald already has real
enums/ADTs (plan 52) and `Result[T,E]` with `?`-propagation (plan 53).
What's missing is purely a *convention*: nearly every plan in the 91-191
batch that can fail returns `Result[T, String]`, verified directly
against the batch's own text rather than assumed —

- Plan 118: `Json.parse(s: String): Result[JsonValue, String]`
- Plan 122: `Regex.compile(pattern: String): Result[Regex, String]`
- Plan 53's own worked examples (`parse_int`, `try_parse`) are
  themselves `Result[Int64, String]`-shaped, meaning the batch inherited
  this shape from the very type's own introductory example, not by
  accident in any one plan.

A caller receiving any of these can pattern-match `Ok`/`Err` (real,
already-shipped machinery) but can never branch on *what kind* of
failure occurred short of parsing the message string itself — exactly
the gap Rust's `std::error::Error`/downcast ecosystem, Go's
`errors.Is`/`errors.As`, and Python's exception-class hierarchy all
exist to close, and which this project's own `Result[T,E]` is already
structurally capable of closing the same way, today, with zero new
compiler work.

This plan does three things: (1) defines the convention precisely, so
every future domain-plan author has one canonical shape to follow
rather than re-deriving it per-domain; (2) extends plan 92's FFI/ABI
additively so a native error's real classification survives the
Rust-to-Emerald boundary instead of being flattened to a string before
it gets there; (3) retrofits two already-shipped plans (118, 122) as
real, concrete, disclosed-breaking-change worked examples, not just an
abstract recommendation.

## Concrete proof this plan targets

**Before** (plan 118's own shipped, current shape):

```ruby
result: Result[JsonValue, String] = Json.parse(input)
case result
when Ok(doc) do
  puts doc.to_s
when Err(msg) do
  puts "parse failed: #{msg}"
end
```

`msg` here is `serde_json::Error`'s own `Display` text verbatim (plan
118's own disclosed behavior) — a caller who wants to distinguish "the
input was truncated mid-object" from "the input had a stray comma" has
no way to do so short of matching substrings against an upstream
crate's own error-message wording, which is not a public API contract
`serde_json` makes any stability promise about.

**After** (this plan's proposed retrofit):

```ruby
enum JsonError
  Syntax(String)
  UnexpectedEnd
  Other(String)
end

result: Result[JsonValue, JsonError] = Json.parse(input)
case result
when Ok(doc) do
  puts doc.to_s
when Err(JsonError::UnexpectedEnd) do
  puts "input was truncated"
when Err(JsonError::Syntax(detail)) do
  puts "syntax error: #{detail}"
when Err(JsonError::Other(detail)) do
  puts "parse failed: #{detail}"
end
```

The Rust side (`crates/emerald-rt/src/json.rs`) classifies via
`serde_json::Error::classify()` — a real method on the real, already-
vendored crate, not invented for this plan — mapping its
`Category::Eof` to `JsonError::UnexpectedEnd`, `Category::Syntax` to
`JsonError::Syntax(msg)`, and `Category::Io`/`Category::Data` to
`JsonError::Other(msg)` for this v1 scope (both are real categories
`serde_json` exposes; this plan does not claim they need finer-grained
Emerald-visible variants yet — see Decision log), then calls
`emerald_rt_result_err_tagged(tag, msg)` with the corresponding `i32`
tag instead of plan 92's original untagged `emerald_rt_result_err(msg)`.

## Decision log

- **Per-domain error enums, never one shared cross-domain enum.**
  Considered and rejected a single global `EmeraldError` ADT with one
  variant per domain (`EmeraldError::Json(...)`, `EmeraldError::Regex
  (...)`, ...). Rejected because it makes every domain's error surface
  a compile-time dependency of every *other* domain's — adding a new
  `Gzip.compress` failure mode would mean editing a type file every
  unrelated already-shipped plan's generated code also references,
  and because Emerald's own domains (parsing formats, compression,
  networking, crypto) share no real semantic error taxonomy worth
  forcing into one ADT (a `JsonError::Syntax` and a `GzipError::
  CorruptStream` are not meaningfully "the same kind of thing" a
  caller would ever want to match generically across both). This
  mirrors Rust's own `std` choice not to have one universal `Error`
  enum either, despite `std::error::Error` giving it the trait
  machinery to unify them if it wanted to.
- **`Other(String)` is a required, not optional, escape-hatch variant
  on every `<Domain>Error`.** No domain plan can enumerate, up front,
  every failure mode a wrapped third-party crate is capable of
  producing — `serde_json`, `regex`, `flate2`, and every future
  wrapped crate are free to add new error conditions in a later
  version this project pins to. Forcing full enumeration up front
  either blocks on exhaustiveness this plan cannot actually verify, or
  produces a false sense of completeness. `Other(String)` keeps the
  enum exhaustive from Emerald's own `case`/`when` perspective (a real,
  compiler-checked exhaustive match, plan 11's own exception-handling
  precedent) while staying honest that not every native failure has
  been individually named yet — a domain's variant list is expected to
  grow over time, promoting real observed `Other(...)` cases to named
  variants, the same evidence-driven growth `DEPENDENCIES.md` (plan 95)
  already models for crate additions.
- **Cross-domain composition (a caller using both `Json` and `Toml` in
  one `?`-propagating function) is a real, disclosed ergonomic gap this
  plan does not solve.** Rust's own answer to this exact problem is
  `From`/`Into`-based automatic error coercion at the `?` operator
  (`impl From<JsonError> for MyAppError`) — Emerald's `?`-propagation
  (plan 53) has no equivalent automatic-conversion mechanism today, so
  a function mixing two domains' typed errors must either narrow both
  into a shared enum by hand at each call site (verbose, but correct
  and available today with zero new compiler work) or keep using
  `Result[T, String]` at that specific composition boundary as an
  explicit, deliberate opt-out. This plan does not propose an
  `Into`-style auto-coercion mechanism — that is a real, separate
  language-level feature request this plan flags but does not scope,
  since it is compiler surface, not a per-domain convention.
- **The FFI extension is additive, not a breaking change to plan 92.**
  `emerald_rt_result_err(msg)` is kept, unedited, and still the correct
  choice for any domain that has not adopted this convention — nothing
  in plan 92's own shipped, tested behavior changes. `tag`'s meaning is
  owned entirely by the domain that assigns it (no global tag registry
  file, deliberately, to avoid the same shared-coupling problem the
  per-domain-enum decision above already avoids) — the Emerald-side
  static-dispatch arm for each domain (plan 45's `File.read`-style
  hard-coded-arm shape, already precedent for `Json`/`Regex`/`Gzip`
  namespaces) is the only place that needs to know how to turn a
  domain's own `i32` tags back into that domain's own enum variants.
- **Why plans 118 and 122 specifically, not two unimplemented plans.**
  A retrofit against already-shipped, already-tested code is a real,
  falsifiable proof this convention actually works end to end (compiles,
  round-trips through the FFI boundary, matches correctly in `case`/
  `when`) — retrofitting two plans that don't exist as working code yet
  would only prove the convention reads plausibly, the same "asserted,
  not verified" gap this project's own `AGENTS.md` names as a past,
  real failure mode (plan 68). `118` and `122` were chosen because both
  already show a clean `Result[T, String]` shape in their own Concrete
  Proof sections (quoted above) and both wrap a crate (`serde_json`,
  `regex`) with a real, inspectable native error type to classify from
  — not chosen because they were the easiest, but because they are
  each independently a complete, representative worked example (a
  parser error and a compile-time validation error, two genuinely
  different failure shapes).
- **Out of scope.** No change to plan 53's `Result[T,E]`/`?`-propagation
  mechanism itself (it already supports any `E`, including an enum —
  verified against plan 53's own text, `Result[Int64, String] or
  Result[Int64, IoError]` is cited directly in that plan's own worked
  example). No retrofit of any plan beyond 118/122 in this plan's own
  EXECUTE phase — every other not-yet-implemented plan applies this
  convention at its own EXECUTE time per `leaf-checklist-for-future-
  plans`, not as a mass retrofit here. No `Into`/`From`-style automatic
  cross-domain error coercion (see composition bullet above). No change
  to `emerald_rt_result_ok`/the `Ok` path at all — this plan is
  `Err`-path only, since `Ok`'s payload was never string-flattened in
  the first place.

## Update (2026-09-23, same-day execution session): implemented, all five leaves done

Real-checked against actually-vendored crates before writing any code,
per this plan's own instruction not to trust its text alone:
`crates/emerald-rt/Cargo.toml` pins `serde_json = "1"` /
`regex = "1.13"`, resolving in `Cargo.lock` to `serde_json 1.0.151`
and `regex 1.13.1`. `serde_json-1.0.151/src/error.rs`'s own
`Error::classify(): Category` and `pub enum Category { Io, Syntax,
Data, Eof }` match this plan's own text exactly — no correction
needed there. `regex-1.13.1/src/error.rs`'s own `#[non_exhaustive]
pub enum Error` has exactly two variants, `Syntax(String)` and
`CompiledTooBig(usize)` — confirming the plan's own fallback ("a
two-variant `Syntax`/`Other` enum is a legitimate v1 scope") is not
just legitimate but the *complete, accurate* picture: `CompiledTooBig`
folds into `Other`, nothing was invented.

**leaf-define-the-convention**: this file's own Concrete Proof and
Decision log above stand as the convention's canonical text — no
change needed beyond this Update section.

**leaf-ffi-tagged-error-export**: `emerald_rt_result_err_tagged(tag:
i32, msg: *const c_char) -> *mut c_void` added to `crates/emerald-rt/
src/lib.rs`, immediately after `emerald_rt_result_err_str`.
`emerald_rt_result_err(msg)` (plan 92) is untouched. The Err payload
it builds is a pointer to a real, `emerald_alloc`-backed two-word enum
block (`[tag: i64][msg: *const c_char]`) — byte-for-byte the same
`EnumLayout` shape `json.rs`'s own `alloc_enum_block`/`JsonValue`
already establishes, so a `<Domain>Error` enum registered in
`emerald-sema`/`emerald-codegen` with matching variant order reads it
like any other compiler-synthesized enum value, zero extra marshaling.
A crate-internal `emerald_rt_result_err_tagged_str(tag, msg: &str)`
sibling was added too, mirroring the existing `emerald_rt_result_err`/
`emerald_rt_result_err_str` relationship, so `json.rs`/`regex.rs` (both
already holding an owned `String`/`&str`) avoid a pointless `CString`
round-trip.

**leaf-retrofit-json-worked-example**: `Json.parse`'s signature is now
`Result[JsonValue, JsonError]`, `JsonError = Syntax(String) |
UnexpectedEnd | Other(String)` — registered as a compiler-synthesized,
non-generic enum in both `emerald-sema` and `emerald-codegen`, the
identical two-crate mirrored-registration shape `JsonValue` itself
already uses. `crates/emerald-rt/src/json.rs`'s `json_parse` classifies
via `serde_json::Error::classify()` exactly as this plan's own
Concrete Proof specifies (`Category::Eof` → `UnexpectedEnd`,
`Category::Syntax` → `Syntax`, `Category::Io`/`Category::Data` →
`Other`) and calls `emerald_rt_result_err_tagged_str`, never the
untagged path. **Disclosed breaking change to plan 118's public
surface**: any `.em` code matching `Err(msg) do ... end` against a bare
`String` no longer type-checks. `examples/json_demo.em` is updated
in place (not a new file) to match — both `Err` arms now nested-match
`JsonError`'s three variants, and the file gained a third negative
proof (`"{\"a\":"` , truncated mid-object) specifically to exercise
`UnexpectedEnd`, alongside the original `Ok` and `Syntax`-error
(`"{\"a\":1,}"`, a real stray-comma syntax error, swapped in for the
original `"{not json"` specifically so the classification is
unambiguous) proofs. `Toml.parse` (plan 119) was deliberately left
untouched — out of this plan's own EXECUTE-phase scope; it still
returns `Result[JsonValue, String]`.

**leaf-retrofit-regex-worked-example**: `Regex.compile`'s signature is
now `Result[Regex, RegexError]`, `RegexError = Syntax(String) |
Other(String)` — the minimum shape, and (per the real crate check
above) the complete one. `crates/emerald-rt/src/regex.rs`'s
`regex_compile` matches on `regex::Error::Syntax(_)` for the `Syntax`
tag and folds everything else (`CompiledTooBig`, and any future
`#[non_exhaustive]` variant) into `Other`, calling
`emerald_rt_result_err_tagged_str`. **Disclosed breaking change to
plan 122's public surface**: same `Err(msg)`-as-bare-`String` break as
JSON above. `examples/regex_dates.em` is updated in place: the
existing success-path `Err` arm now nested-matches `RegexError`, and a
new negative-proof block (`Regex.compile("(unclosed")`) was added,
landing on `Syntax` — this plan's own instruction to verify "a real
failure case," not just a success path, for both retrofits.

**leaf-checklist-for-future-plans**: stands as already drafted in this
file's own todo text above — advisory, unchanged.

### Real, disclosed finding not anticipated by this plan's own text: the flat, global enum-variant namespace

`JsonError` and `RegexError` both declare a variant literally named
`Syntax`, and both declare one literally named `Other` — the exact
same "required `Other(String)` on every domain" convention this plan
itself mandates, applied to a second domain for the first time.
Plan 124's own Decision log (`XmlNode`/`XmlEvent`) already documents
this compiler's enum-variant namespace as "GLOBAL and flat" and
renamed a colliding variant (`Text` → `TextContent`) to avoid it — so
this was checked directly, not assumed safe. Verified, by reading
`check_case`/`build_match_result`'s own real bodies (`emerald-sema`/
`emerald-codegen`), that pattern-match resolution (both `case`'s
legacy AST and `match X do Err(e) do ... end end`'s `E`-typed `Err`
binding) is always scoped to the scrutinee's own **statically-known**
enum name (`classes.get(enum_name)`/`ctx.enums.get(e_name)`) — never a
global ambiguous search across every registered enum. `find_all_
variants`'s own doc comment confirms this is exactly the mechanism
that already lets `Option[Int64]` and `Option[String]` (two distinct
monomorphized enums) safely share `Some`/`None` today. `JsonError`/
`RegexError` sharing `Syntax`/`Other` is the identical, already-proven-
safe case — confirmed empirically too: the full workspace gate below
passed with both enums' shared variant names left exactly as the
Concrete Proof and Decision log above name them, no rename needed
(unlike plan 124's `Text`/`TextContent`, whose own collision was a
different, narrower construction-site ambiguity this Err-binding path
doesn't share).

### Full verification ladder (all clean)

`cargo build --workspace` (clean). `cargo nextest run --workspace`:
1098/1102 passed, 2 skipped, 4 failed — all 4 failures verified
unrelated to this plan, not fixed by this plan (out of its own scope):
`dns_resolution_em_prints_expected_sequence` (no outbound network
access in this sandbox, pre-existing/environment, unrelated to any
code here) and three failures (`every_real_file_under_examples_lints_
clean`, `every_example_file_formats_and_reparses_to_an_equivalent_ast`,
`every_example_file_formatting_is_idempotent`) all traced to a single
real parse error in `examples/set_deque_priority_queue.em` — a file
this session found already present, mid-edit, from a different
concurrently-running plan (193) in this same shared checkout, not
touched by plan 195. Confirmed via `git stash` that these 4 failures
(reduced to exactly the same 3, plus dns) reproduce identically with
this plan's own diff stashed out — genuinely pre-existing, not a
regression this plan introduced. (A first full-parallelism run showed
far more failures across unrelated actor/scheduler/supervisor tests;
re-run at `--test-threads=4` collapsed to the 4 above — this sandbox's
default full parallelism is itself flaky under this many test
binaries, a real, disclosed environment finding, not a code bug.)
`json_demo_em_prints_expected_sequence`/`regex_dates_em_prints_
expected_sequence` (`crates/emerald-cli/tests/examples.rs`) — both
updated to the real, observed output and passing.

`cargo clippy --workspace --all-targets` (clean). `treefmt` (5 files
processed, 0 changed). `cargo audit --ignore RUSTSEC-2023-0071` (exit
0; 5 pre-existing `bitmaps`/`im`/`sized-chunks` unmaintained/unsound
warnings, unrelated to `serde_json`/`regex`, already present before
this plan — no new advisory).

`examples/json_demo.em` and `examples/regex_dates.em` both run end to
end via the real CLI (`cargo run -p emerald-cli -- examples/<file>.em`
then executing the produced binary directly — the legacy bare-file
mode compiles but does not itself execute), exercising a success case
and a real, typed failure case for each domain. `json_demo.em`'s real
output: `Ada` / `missing` / `2` / the full `to_s` round-trip /
`trailing comma at line 1 column 8` (`Syntax`) / `unexpected end of
input` (`UnexpectedEnd`). `regex_dates.em`'s real output: `true` /
`2026-09-21` / `2026` / `21/09/2026 and 08/01/2026` / the real,
multi-line `regex` crate syntax-error text (`Syntax`).
