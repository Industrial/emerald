2026-09-21T20:59:00Z

---
name: Path Globbing
overview: "A `Glob` compiler-known namespace — `Glob.match(pattern): Array[String]` + `Glob.match_count(pattern): Int64` — backed by the `glob` crate (the `rust-lang`-published crate, MSRV 1.63, its own crates.io description: \"Support for matching file paths against Unix shell style patterns\"), giving Emerald `*`/`?`/`[...]`/`**` shell-style pattern matching directly against the real filesystem. A thin, pattern-aware layer on top of plan 144's plain enumeration, not a competing mechanism: plan 144's `Dir.walk` lists everything, unconditionally, and leaves filtering to Emerald source; this plan's `Glob.match` filters natively in Rust, during traversal, so a pattern with a static prefix (`src/**/*.rs`) never even descends into directories a match could not possibly come from — genuinely more than a convenience wrapper around `Dir.walk`, a real efficiency and ergonomics difference plan 144 alone cannot provide."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-glob-crate-and-sorted-match
    content: "Add `glob = \"0.3\"` to `crates/emerald-rt/Cargo.toml`. `emerald_rt_glob_match(pattern: *const c_char) -> *mut *mut c_char` / `emerald_rt_glob_match_count(pattern: *const c_char) -> i64`: call `glob::glob(pattern)`, collect every `Ok(PathBuf)` result into a `Vec<String>` (silently skip `Err` entries — individual permission-denied directories during the walk — see Decision log), sort the vector lexicographically before returning, then hand back the same array-plus-count pair plans 45/144/145/146 already established for every `Array[T]`-returning intrinsic in this batch. Both wrapped in `std::panic::catch_unwind` per plan 91's proven pattern."
    status: pending
  - id: leaf-sema-and-codegen-wiring
    content: "Add the `Glob` compiler-known namespace arm to `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`, matching the established hard-coded-arm shape; declare `emerald_rt_glob_match`/`emerald_rt_glob_match_count` via `module.add_function(..., Some(Linkage::External))`."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/path_globbing_proof.em` (the Concrete Proof below), combining plan 45's `File.write` (to create fixture files) with `Glob.match`. Add `#[test]`s inside `crates/emerald-rt`: one asserting `*.rs` matches only top-level `.rs` files in a fixture directory, not files in subdirectories; one asserting `**/*.rs` matches files at every depth; one asserting the returned array is genuinely sorted (build a fixture whose real directory-read order is not alphabetical — e.g. create `b.txt` before `a.txt` — and assert the returned order is still `a.txt` before `b.txt`, the concrete, falsifiable proof of this plan's own sorting decision, not an assumption about the underlying crate's iteration order)."
    status: pending
isProject: false
---

# Plan 150 — Path Globbing

Plan 144 gave Emerald two ways to enumerate a filesystem tree: a flat,
non-recursive listing (`Dir.entries`) and an unconditional recursive walk
(`Dir.walk`) — both list *everything*, leaving any filtering entirely to
Emerald source, which today has no string-pattern-matching primitive
richer than plan 45's literal-substring `.split`. Shell-style globbing —
`*.rs`, `**/*.em`, `src/[abc]*.rs` — is a distinct, genuinely useful
capability neither of plan 144's two primitives provides on its own: a
program could in principle reimplement `*.rs`-style matching over `Dir.
walk`'s output using nothing but `.split`/`.slice`, but `**`-style
recursive-wildcard matching against an arbitrarily deep tree is real,
non-trivial matching logic worth getting from a dedicated, widely-used
crate rather than hand-rolling in Emerald source or in this plan's own
new Rust code. The `glob` crate is the `rust-lang`-published, long-
standing choice for exactly this (verified this session: its own
crates.io listing states "Support for matching file paths against Unix
shell style patterns", MSRV 1.63.0, actively listed with a current
release as of mid-2026).

## Concrete proof this plan targets

```ruby
File.write("plan150_demo/b.txt", "b")
File.write("plan150_demo/a.txt", "a")
File.write("plan150_demo/nested/c.txt", "c")
File.write("plan150_demo/nested/d.rs", "d")

top: Array[String] = Glob.match("plan150_demo/*.txt")
n: Int64 = Glob.match_count("plan150_demo/*.txt")
puts n
puts top[0]
puts top[1]

deep: Array[String] = Glob.match("plan150_demo/**/*.rs")
m: Int64 = Glob.match_count("plan150_demo/**/*.rs")
puts m
puts deep[0]
```

Expected output, in order (run from a fresh temporary working directory):
```
2
plan150_demo/a.txt
plan150_demo/b.txt
1
plan150_demo/nested/d.rs
```

Trace: `plan150_demo/*.txt` matches only the two top-level `.txt` files —
`nested/c.txt` is one directory too deep for a single `*` to reach, and
`nested/d.rs` doesn't match the `.txt` extension regardless — proving
single-`*` globbing does not silently recurse. The two matched paths come
back sorted (`a.txt` before `b.txt`) even though `b.txt` was written to
disk first — a real, deliberate proof of this plan's own sort-before-
returning decision, not an accident of directory-read order.
`plan150_demo/**/*.rs` matches only `nested/d.rs`, proving `**` reaches
into a subdirectory a single `*` could not.

## Decision log

- **`Glob.match` is a pattern-filtered, natively-traversing layer on top
  of plan 144's `Dir.walk`, not a duplicate of it — the relationship is
  stated precisely, not left implicit.** `Dir.walk` (plan 144, via
  `walkdir`) always descends every directory in the tree, unconditionally
  — filtering, if any, happens afterward, in Emerald source. `Glob.match`
  (this plan, via the `glob` crate) descends only directories a pattern's
  own static prefix could still match — a pattern like `src/models/*.rs`
  never even opens `src/views/`, since `glob`'s own matching walk knows
  `views` can never satisfy the literal `models` path segment. This is a
  genuine efficiency difference on a large tree, not merely a shorter
  spelling for "call `Dir.walk` then filter in a loop" — and it is also
  what gives Emerald real, familiar `*`/`?`/`[...]`/`**` glob syntax at
  all, which plan 144's plain enumeration primitives have no concept of.
  A program that needs a predicate `Dir.walk`'s glob-unaware output
  cannot express (e.g. "every file modified in the last hour", using
  plan 144's `FileMetadata`) still reaches for `Dir.walk` plus manual
  filtering — the two primitives serve genuinely different filtering
  needs, one pattern-syntax-based, one arbitrary-predicate-based.
- **Results are sorted lexicographically by this plan's own Rust code
  before crossing the FFI boundary — a deliberate correction of the
  underlying crate's own unspecified iteration order, verified this
  session, not assumed.** A real forum thread ("Sorting Glob Result",
  Rust Users Forum) exists specifically because `glob`'s own iteration
  order follows the OS's real directory-read order, which is not
  alphabetical on every filesystem and is not part of the crate's own
  documented contract — callers who want sorted output are expected to
  sort it themselves. This plan does that sorting once, inside
  `emerald_rt_glob_match`, so every Emerald program calling `Glob.match`
  gets deterministic, reproducible output regardless of the underlying
  filesystem's own physical directory-entry order — the same
  determinism this plan's own Concrete Proof depends on and directly
  tests (see the leaf above testing a deliberately-non-alphabetical
  write order).
- **A directory `glob` cannot read (permission denied mid-walk) is
  silently skipped, not surfaced as a partial-failure error — a real,
  disclosed simplification.** `glob::glob(pattern)`'s iterator yields
  `Result<PathBuf, GlobError>` per entry; a `GlobError` most commonly
  means one specific subdirectory within the pattern's scan couldn't be
  read (permissions, a race where a directory was removed mid-walk).
  This plan drops those entries rather than aborting the whole match or
  inventing a partial-result-plus-errors return shape — the same
  "collect what you can, don't build new machinery for a rare partial-
  failure case" posture plan 45 already took toward `.split`'s own
  simplifications. A caller who needs to know a scan was incomplete has
  no signal under this plan — a real, accepted, narrow gap.
- **`Array[T]`'s companion-count pattern is the fifth confirmed sighting
  in this batch — no new discussion needed, cited to plans 45/144/145/
  146 for the full justification.**
- **`Glob.match`'s pattern syntax is exactly the `glob` crate's own
  syntax, not a new dialect this plan defines.** `*` matches any
  sequence of characters except a path separator, `**` matches any
  number of path segments (including zero), `?` matches exactly one
  character, `[...]`/`[!...]` match a character class — this plan adds
  no translation layer and no Emerald-specific pattern extensions; an
  Emerald `Glob.match` pattern behaves identically to the same string
  passed to `glob::glob` directly, keeping this plan's own surface exact
  and its behavior fully specified by an external, already-documented
  contract rather than a new one this plan would have to maintain.
- **Out of scope.** Glob matching against an in-memory list of strings
  with no real filesystem behind it (a distinct, smaller, pure-string-
  matching feature `globset`/`fast-glob` are better suited to, and not
  needed by this plan's own filesystem-facing Concrete Proof); brace
  expansion (`{a,b}.rs`, a real shell feature the plain `glob` crate does
  not implement at all — adding it would mean a different, heavier
  crate or hand-rolled pre-expansion, neither justified by this plan's
  scope); case-insensitive matching (platform-dependent and not exposed
  by `glob`'s own basic API); any change to plan 144's `Dir`/`Path`
  namespace or to `runtime/emerald_runtime.c`.
