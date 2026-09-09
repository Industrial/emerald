---
name: Incremental, Query-Based Compilation
overview: "Wrap the existing parse -> type-check -> codegen pipeline behind memoized, content-hash-keyed queries (Salsa/rustc-style, named explicitly) so a second, unchanged invocation skips recompilation entirely — a pure speed layer with zero observable diagnostic/behavior change, scoped to whole-file granularity, and wired directly into plan 47's REPL and plan 46's multi-file `emerald build`."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-query-cache-core
    content: "A generic, content-hash-keyed QueryCache in crates/emerald-driver/src/cache.rs wrapping check()/compile()'s existing stage calls unchanged — a cache MISS always falls back to calling the real emerald-sema/emerald-codegen function verbatim; compiler-binary fingerprint folded into every key"
    status: pending
  - id: leaf-require-graph-cache-keys
    content: "Extend plan 23's resolve_program to return each file's own individual content hash alongside the merged Program, so type_check/codegen's cache key is a hash-of-hashes over the real require closure — touching one file invalidates only entry points that transitively require it"
    status: pending
  - id: leaf-verbose-flag-and-consumer-wiring
    content: "--verbose-cache (HIT/MISS reporting, since the payoff is otherwise invisible in stdout) threaded through emerald-cli's single-file mode, plan 46's emerald build/run, and plan 47's emerald repl (plus repl's --history-cache for transcript replay)"
    status: pending
isProject: false
---

# Plan 48 — Incremental, Query-Based Compilation

This is plan 48, the first of the 48-57 batch implementing the "Beyond
the Ceiling" analysis in full — ten independent sibling plans, none of
them conceding Emerald's identity constraints (no `method_missing`/
`eval`/`send`/reflection, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing GC, no runtime
reflection). The analysis split into two tracks: pure compiler-
implementation improvements that touch no language semantics, and an
actor-model/algebraic-data-type/`Result[T,E]` pillar. This plan is
squarely the former — it adds not one `Expr`/`Stmt`/`Item` variant, not
one sema rule, not one new codegen path; it only changes *how many
times* the existing, unchanged pipeline functions get called for the
same input. Like every prior post-v1 batch (17-27, 28-35, 36-47), this
is post-v1 scope; it is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
and that document (along with this batch's own eventual summary) is
updated once, separately, after all ten of plans 48-57 are authored —
this plan does not touch `plan-of-plans.md` or any other plan file.

Concrete proof this plan targets — a three-file program compiled twice
unchanged, then one file touched:

```
$ emerald-cli examples/query_cache_demo/main.em -o /tmp/demo --verbose-cache
[cache] parse   main.em     MISS
[cache] parse   helpers.em  MISS
[cache] parse   utils.em    MISS
[cache] check   main.em     MISS
[cache] codegen main.em     MISS
$ emerald-cli examples/query_cache_demo/main.em -o /tmp/demo --verbose-cache
[cache] parse   main.em     HIT
[cache] parse   helpers.em  HIT
[cache] parse   utils.em    HIT
[cache] check   main.em     HIT
[cache] codegen main.em     HIT
$ # edit one byte inside helpers.em (a real content change, not just mtime)
$ emerald-cli examples/query_cache_demo/main.em -o /tmp/demo --verbose-cache
[cache] parse   main.em     HIT
[cache] parse   helpers.em  MISS
[cache] parse   utils.em    HIT
[cache] check   main.em     MISS
[cache] codegen main.em     MISS
$ /tmp/demo
14
```
Both linked binaries print the identical `14` on every run — the cache
never changes *what* is produced, only whether LLVM and `emerald-sema`
are invoked again to produce it. `utils.em`'s own parse query stays a
HIT even after `helpers.em` changes (it is untouched, and nothing that
requires it also requires `helpers.em` in a way that would invalidate
it); `main.em`'s `check`/`codegen` queries MISS because its merged
program's cache key is a hash over its *entire* require closure, which
now includes `helpers.em`'s new content.

## Decision log

- **Verified this session: the pipeline has exactly zero caching
  today, at any stage.** `crates/emerald-cli/src/main.rs`'s `main`
  builds one `id_effect::Effect` chain — `parse_stage` (calls
  `emerald_parser::parse_named` directly), `check_stage` (calls
  `emerald_sema::check_program` directly), `codegen_stage` (calls
  `emerald_codegen::compile_to_object` directly), `link_stage` (shells
  out to `cc`) — and `run_blocking` executes that chain unconditionally
  on every invocation; nothing is written to disk except a
  process-scoped temp `.o` file and the embedded runtime archive, both
  deleted the moment `link_stage` finishes (`std::fs::remove_file`,
  called on both success and failure paths). There is no
  `.emerald/cache/`, no persisted AST, nothing. This plan wraps each of
  those four call sites' *first three* stages behind a memoized query
  (see the "no `link_stage` caching" bullet below) — a cache MISS
  always falls back to calling `emerald_parser::parse_named`/
  `emerald_sema::check_program`/`emerald_codegen::compile_to_object`
  exactly as `main.rs` calls them today, with the identical arguments
  and identical return values. This plan changes **zero** observable
  compiler behavior or diagnostic text — `leaf-query-cache-core`'s own
  acceptance criteria require proving byte-identical output between the
  cached and uncached code paths, not just asserting it.
- **This is the point inception's own `salsa` gate was written for, and
  the precedent is named explicitly rather than smuggled in as an
  unnamed "caching layer."** `history/2026-09-08T173600Z-inception.md`
  §14.5 names the Rust `salsa` crate directly ("incremental, on-demand
  computation with memoized tracked functions and dependency
  tracking") and gates it: "do not add unless it materially simplifies
  the architecture." Plans 13, 17, and 23 (verified this session, all
  three) each declined it for exactly the reason the gate anticipates —
  there was no concrete, named consumer yet that repeatedly re-ran the
  same pipeline over unchanged input. That consumer now exists twice
  over: plan 47's REPL re-runs `check`/`compile` on `prelude + line`
  every single line, and plan 46's `emerald build`/`run` re-runs the
  full pipeline on every invocation regardless of whether anything
  changed (its own worked example's comment says so plainly:
  `# emerald.lock unchanged -> resolution skipped, only recompiled`).
  This plan is the query-based architecture rustc itself uses
  internally (`rustc_query_system`) and the design pattern the `salsa`
  crate generalizes — named explicitly, per the task brief — applied by
  hand, as three narrow memoized functions, not by adding the `salsa`
  crate as a dependency. That distinction is deliberate: `salsa`'s own
  value proposition is fine-grained, intra-item incrementality via a
  derive-macro-driven query database; this plan's whole-file scope (see
  next bullet) doesn't need that machinery's complexity, and pulling in
  a crate whose main feature this plan explicitly declines using would
  be adding a dependency for a fraction of what it offers.
- **Scoped to whole-file granularity — re-check/re-codegen an entire
  file's containing program when anything in its require closure
  changes — not the fine-grained, intra-file incrementality real Salsa/
  rustc use at full generality (memoizing individual function bodies,
  re-checking only the one function whose text changed).** That finer
  lattice needs per-item cache keys threaded through `emerald-sema`'s
  and `emerald-codegen`'s internals (both currently take one `&Program`
  and walk it as a whole — verified against `check_program`'s signature
  at `crates/emerald-sema/src/lib.rs` line ~1764 and `emerald-codegen`'s
  module-level `compile_to_object` entry point), a materially larger
  redesign of both crates' internal data flow, not a call-site wrapper.
  This plan is real, disclosed, deliberately narrower scope: a whole
  file (or, for a multi-file program, the file's entire transitive
  require closure) is the unit of cache invalidation. Finer-grained
  intra-file incrementality is real, valuable, disclosed future work —
  a distinctly larger project, not an oversight here.
- **Cache keys are content hashes of raw source bytes, not hashes of
  the in-memory AST — a choice forced by, and consistent with, an
  existing deliberate decision already in the code, not a new one this
  plan invents.** Verified this session: `crates/emerald-parser/src/
  ast.rs`'s own comment above `Expr`'s derive list states plainly that
  `Eq`/`Hash` are "intentionally not derived" because `Expr::Float(f64)`
  can't satisfy `Eq`'s reflexivity and "nothing in this workspace needs
  `Expr`/`Stmt`/etc. as a `HashMap`/`HashSet` key." Reopening that to
  hash `Program` structurally for this plan's own purposes would
  contradict a real, stated design decision instead of respecting it.
  Since `emerald_parser::parse_named` is a pure, deterministic function
  of `(source bytes, name)` and `emerald_sema::check_program`/
  `emerald_codegen::compile_to_object` are pure functions of the
  resulting `Program`, hashing the *source bytes* is an equally valid
  cache key for all three queries — `type_check`'s and `codegen`'s
  cached results are exactly as correct keyed on "the bytes that
  produced this `Program`" as on the `Program` value itself, with zero
  new derives anywhere in `emerald-parser::ast`.
- **Every cache key is salted with a content hash of the running
  compiler's own binary (`std::fs::read(std::env::current_exe())`),
  computed once per process and folded into every query's key.**
  Concrete, easy-to-get-wrong correctness pitfall this plan's own
  worked example doesn't show but its acceptance criteria must prove:
  without this, rebuilding `emerald-cli` with a codegen change (a
  hypothetical bug fix, or simply a new local build during this
  compiler's own development) and then re-running against an
  unchanged Emerald source file would silently reuse a stale cached
  `.o` file compiled by the old binary — a real miscompilation risk a
  pure source-content key can't catch, since the source didn't change,
  only the compiler that translates it did. `leaf-query-cache-core`'s
  acceptance criteria require proving this directly (via an injectable
  fingerprint seam in tests, not a full rebuild-and-rerun), the same
  "state it and prove it, don't just assert it" discipline plan 31's
  swap-ordering pitfall already established for this project.
- **No hash crate exists anywhere in this workspace today** (verified
  this session: zero matches for `sha2`/`blake3`/any hashing crate name
  across every `crates/*/Cargo.toml`) **— `blake3` is added as a new,
  disclosed dependency to `crates/emerald-driver/Cargo.toml`.** Chosen
  over `sha2`/`sha1` for raw throughput on the repeated whole-file
  hashing this plan does on every invocation (BLAKE3 is materially
  faster than SHA-2 at this workspace's file sizes, per its own
  published design goals) and over a non-cryptographic hash
  (`xxhash`/`fnv`) because a cache-key collision here would mean a
  wrong program silently reusing another program's cached object file —
  a correctness-sensitive use, not a hash-table load-factor concern,
  where a cryptographic hash's collision resistance is the right
  default even though nothing here is adversarial.
- **Multi-file cache keys extend plan 23's own `resolve_program`
  design, and depend on plan 17's driver extraction the same way plans
  46, 47, and 23 itself already disclosed.** Verified this session:
  `crates/emerald-parser/src/ast.rs`'s `Item::Require` doc comment
  states directly that "`emerald-driver`'s `resolve_program` (plan 17,
  not yet extracted in this codebase) is meant to strip every
  `Item::Require`" — confirmed also by `crates/emerald-cli/src/main.rs`
  containing no require-resolution logic at all today (a single
  `std::fs::read_to_string(source_path)` call, no directory walk) and
  by the workspace root `Cargo.toml` still listing exactly five
  members, no `emerald-driver`. This plan's `leaf-require-graph-cache-
  keys` builds directly on `resolve_program`'s design (DFS over
  `Item::Require`, canonical-path `visited` dedup, in-progress-stack
  cycle detection) by having it additionally return each visited file's
  own content hash alongside the merged `Program` it already produces —
  a small, additive change to what `resolve_program` returns, not a
  redesign of how it walks the graph. If plan 17's driver extraction
  and plan 23's `leaf-driver-resolution` haven't landed by this plan's
  own execution time, `leaf-require-graph-cache-keys`'s acceptance
  criteria are unverifiable until they do — stated here rather than
  silently assumed, the identical sequencing posture plans 46 and 47
  already adopted for the same dependency.
- **The REPL benefits from this cache along exactly one honest axis:
  full-transcript replay, not incremental single-session typing — and
  that limit is structural, not an implementation shortfall.** Plan
  47's REPL session is `prelude` (a growing string of already-accepted
  declarations) plus one new line, compiled as `prelude + line_text` on
  every line (verified against plan 47's own Decision log). Because
  `prelude` only ever grows, the full candidate text compiled for line
  N is never byte-identical to the text compiled for any earlier line —
  under this plan's whole-file content-hash keying, that guarantees a
  cache MISS on `type_check`/`codegen` for every new line in a single,
  organically-typed interactive session, by construction. What *does*
  get a real hit: (1) resubmitting the exact same line against the
  exact same prelude (e.g. re-running a previous expression), a real if
  narrow win; and (2), the concrete integration point this plan adds
  for plan 47 — `emerald repl --history-cache <dir>` persists the query
  cache to a stable, named directory instead of a throwaway per-session
  temp directory, so replaying an *unmodified* transcript (a saved
  demo script, a CI smoke test, restoring a crashed session by
  re-pasting its history) hits every previously-seen line's `check`/
  `codegen` cache and only compiles whatever is genuinely new past the
  last matching line. This is disclosed precisely as what it is — a
  transcript-replay optimization — not oversold as solving the REPL's
  real, separate pain point (recompiling an ever-growing unchanged
  prefix during live typing), which needs the finer-grained
  incrementality this plan explicitly declines above.
- **`emerald build`/`run` (plan 46) get the strongest, least caveated
  win in this plan, because their unit of work — a package's own entry
  file plus its `require`d dependencies — is exactly this plan's cache
  granularity, and each successive `build`/`run` invocation of an
  unmodified project compiles the identical source closure every
  time.** Plan 46's own worked example already shows `emerald.lock`
  skipping *dependency resolution* on an unchanged manifest while its
  own inline comment admits recompilation still happens every time
  regardless (`# ... -> resolution skipped, only recompiled`); this
  plan's cache sits one layer beneath that lockfile check and closes
  the gap that comment leaves open, without touching `emerald.lock`,
  `manifest.rs`, or dependency resolution at all — `leaf-verbose-flag-
  and-consumer-wiring`'s cache root (`.emerald/cache/`) is a sibling
  directory to plan 46's own `.emerald/deps/` convention, reusing its
  established `.emerald/` namespace rather than inventing a second one.
- **`link_stage` (the final `cc` invocation) is explicitly not
  cached.** Caching it would need its own key surface (the object
  file's hash, the embedded runtime archive's version, and the exact
  linker flags) and its own artifact store (a full linked executable,
  not a `.o`), for a stage plan 47 already measured informally as a
  small fraction of a `hello.em`-sized build's total time compared to
  LLVM's `-O3` codegen. A real, disclosed scope cut — every cached
  invocation in this plan's own worked example still pays one `cc`
  call, just never a repeated `emerald-sema`/LLVM call.
- **Out of scope: disk cache eviction/garbage collection.**
  `.emerald/cache/` only ever grows in this plan — no `emerald clean`,
  no size-based LRU, no time-based expiry. A real, disclosed gap for
  long-lived projects with many edits, deliberately deferred: proving
  the memoization mechanism itself is correct doesn't require solving
  cache-directory hygiene, and an eviction policy is comparatively
  easy, unglamorous follow-up work once real usage shows what eviction
  strategy actually matters in practice.
- **Out of scope: a shared/remote build cache** (an `sccache`-style
  cache shared across machines or CI runners). This plan's cache is
  local-filesystem-only, keyed in part by the running compiler
  binary's own bytes (see above) — a design that would need real
  rethinking (a versioned, portable key scheme independent of any one
  machine's exact binary) to be safely shared across different builds
  of the compiler, which is a distinctly larger, separate project.

## Leaf: leaf-query-cache-core

### 1. Context
- Why: no memoization of any kind exists in the pipeline today (see
  Decision log) — every invocation of `emerald-cli` (and, once they
  exist, `emerald-lsp`/`emerald-mcp`/`emerald repl`/`emerald build`)
  redoes parsing, sema, and LLVM codegen from scratch regardless of
  whether the input changed since the last invocation.
- Target state: `crates/emerald-driver/src/cache.rs` (this plan assumes
  plan 17's `crates/emerald-driver` already exists with its `check`/
  `compile` entry points — if not, extracting it first is a
  prerequisite this plan does not redo, per the Decision log). A
  `QueryCache` struct holding a cache-root `PathBuf` and a
  `compiler_fingerprint: [u8; 32]` (BLAKE3 hash of
  `std::env::current_exe()`'s bytes, computed once at construction,
  injectable via a test-only seam for `leaf-query-cache-core`'s own
  fingerprint-invalidation test). Three methods: `parse_query(path,
  source) -> Result<Program, DriverError>` (keyed by `hash(source
  bytes)`, memoized in an in-process `HashMap` only — not persisted to
  disk, since parsing is cheap and a fresh process has no way to reuse
  a serialized `Program` without adding `serde`/`Hash` derives the
  codebase has deliberately avoided, see Decision log); `type_check_
  query(key: CacheKey, program: &Program) -> Result<(), Vec<String>>`
  persisted at `<root>/check/<key>.txt` (empty file = pass, one
  diagnostic message per line = the prior failure's rendered text, both
  written using nothing beyond `emerald_sema::Diagnostic`'s existing
  `message: String` field — no serde needed); `codegen_query(key:
  CacheKey, program: &Program, obj_path: &Path) -> Result<(), String>`
  persisted at `<root>/obj/<key>.o` (a HIT copies the cached `.o` bytes
  to `obj_path` without invoking `emerald_codegen::compile_to_object`
  at all; a MISS calls it unchanged, then copies its output into the
  cache for next time). Every key additionally folds in
  `compiler_fingerprint`, so `CacheKey` is really `blake3(fingerprint ||
  input_hash)`.

### 2. Acceptance Criteria
1. Two calls to `type_check_query`/`codegen_query` with identical
   `(key, program)` in two separate process invocations: the second
   reports HIT and never calls `emerald_sema::check_program`/
   `emerald_codegen::compile_to_object` (verified via a call-counting
   test double, not just by timing) — and both invocations' actual
   results (pass/fail, object file bytes) are byte-identical.
2. Deleting or corrupting `<root>/obj/<key>.o` between two runs causes
   the second run to report MISS and fall back to a real, correct
   `compile_to_object` call rather than erroring — a cache hit is
   strictly an optimization, never a correctness dependency, proven by
   this exact failure-injection test.
3. Changing one byte of `source`/`program`'s content changes the
   computed key and forces MISS on the very next call, proven directly
   against the hashing function's output, not just observed indirectly.
4. Overriding the injectable `compiler_fingerprint` seam between two
   otherwise-identical calls forces MISS on both `type_check_query` and
   `codegen_query` — the compiler-rebuild correctness pitfall from the
   Decision log, proven directly.
5. Running every existing test program in `crates/emerald-sema/src/
   lib.rs`'s test module (the `HELLO_EM`/`POINT_EXAMPLE`/
   `INHERITANCE_EXAMPLE`/etc. corpus) through `type_check_query` twice
   produces the identical `Ok(())`/`Err(diagnostics)` outcome both
   times, with the second call's diagnostic text byte-identical to the
   first's — proof of zero observable behavior change across the whole
   existing corpus, not a hand-picked example.

### 3. File & Module Structure
- **Create:** `crates/emerald-driver/src/cache.rs`
- **Modify:** `crates/emerald-driver/src/lib.rs` (routes `check`/
  `compile` through `QueryCache` instead of calling
  `emerald_sema::check_program`/`emerald_codegen::compile_to_object`
  directly), `crates/emerald-driver/Cargo.toml` (adds `blake3`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass, incl. HIT/MISS, corruption-fallback, content-change, and fingerprint-change tests | agent-claimed-locally |
| Corpus regression | `cargo test -p emerald-sema` | all existing tests unaffected (this leaf touches no `emerald-sema` code) | agent-claimed-locally |

---

## Leaf: leaf-require-graph-cache-keys

### 1. Context
- Why: `leaf-query-cache-core`'s keys are correct for a single, file-
  free source string, but plan 23's `Item::Require` (already parseable
  per `crates/emerald-parser/src/ast.rs`, verified this session) means
  a real program's `type_check`/`codegen` input is a *merged* `Program`
  spliced from a whole require closure — a single file's own content
  hash is the wrong key for that merged unit, since it ignores every
  file it `require`s.
- Target state: `resolve_program` (plan 23's design, extended here)
  returns `(Program, Vec<(PathBuf, CacheKey)>)` instead of bare
  `Program` — the second element is every file it visited, in
  first-visit order, already deduplicated by canonical path via its
  existing `visited` set (unchanged algorithm — this leaf only adds a
  hash alongside each entry it already tracks). `check`/`compile`
  compute the merged key as `blake3(fingerprint || hash_1 || hash_2 ||
  ... || hash_n)` over that ordered list and pass it to `type_check_
  query`/`codegen_query`; each individual file's own `parse_query` call
  is keyed by its own single-file hash exactly as `leaf-query-cache-
  core` already does, so an untouched file's parse stays a HIT even
  when a sibling required file changes.

### 2. Acceptance Criteria
1. A three-file program (`main.em` requiring both `helpers.em` and
   `utils.em`, mirroring plan 23's own diamond-shaped test fixture
   shape) compiled twice with no edits: `parse_query` reports HIT for
   all three files and `type_check_query`/`codegen_query` report HIT
   for `main.em`'s merged program on the second run.
2. Editing one byte of `helpers.em` only, then rebuilding: `parse_
   query` reports MISS for `helpers.em` and HIT for `main.em` and
   `utils.em`; `type_check_query`/`codegen_query` report MISS for
   `main.em`'s merged program — the literal "touch one file, only it
   (and anything requiring it) recompiles" proof this plan's own worked
   example depends on.
3. A second, independent entry point in the same cache root that
   requires only `utils.em` (never `helpers.em`) keeps its own `type_
   check_query`/`codegen_query` at HIT across the same `helpers.em`
   edit from AC2 — proof that invalidation is scoped to the real
   require graph, not a blanket "any file anywhere changed" rule.
4. Regression: `cargo test -p emerald-driver`'s existing `resolve_
   program` tests (2-file real run, diamond dedup, cycle rejection,
   missing-file error — plan 23's `leaf-driver-resolution` suite)
   all pass unmodified except for the return-type change itself; the
   resolution algorithm's behavior (which files get merged, in what
   order, cycle/missing-file error text) is byte-for-byte unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-driver/src/lib.rs` (`resolve_program`'s
  return type and its two call sites inside `check`/`compile`)
- **Modify:** `crates/emerald-driver/src/cache.rs` (the hash-of-hashes
  combinator function)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass: diamond unchanged-rebuild HIT, single-file-touch selective MISS, independent-entry-point isolation, unmodified resolution-algorithm regressions | agent-claimed-locally |

---

## Leaf: leaf-verbose-flag-and-consumer-wiring

### 1. Context
- Why: `leaf-query-cache-core`/`leaf-require-graph-cache-keys` build a
  cache with no visible effect on `stdout` — this plan's entire payoff
  (fewer LLVM invocations) is otherwise invisible to a user or to this
  plan's own acceptance criteria without a way to observe HIT vs. MISS
  from outside the process; and neither plan 46's `build`/`run`
  subcommands nor plan 47's `repl` call the cache at all yet, since
  both were authored (and, if executed first, would be built) against
  today's uncached pipeline.
- Target state: a `--verbose-cache` flag recognized by `emerald-cli`'s
  existing single-file mode, plan 46's `emerald build`/`emerald run`
  subcommands, and plan 47's `emerald repl`, each printing one `[cache]
  <query> <label> HIT|MISS` line to stderr per query call, in the exact
  format this plan's own worked example shows. Plan 46's `build`/`run`
  pass `.emerald/cache/` (a sibling of its own established
  `.emerald/deps/` directory) as the cache root. Plan 47's `repl`
  defaults to a throwaway per-process-group temp directory (freed with
  the session, matching its own subprocess-per-line model) but accepts
  `--history-cache <dir>` to point at a stable, reusable directory
  instead — the mechanism the Decision log's transcript-replay bullet
  depends on. Wherever plan 46's/47's own call site into the pipeline
  currently lives by the time this leaf executes (inside `emerald-cli`
  directly, or via `emerald-driver`, depending on which of plans 17/46/
  47 landed first — a real, disclosed ambiguity this leaf resolves by
  wiring whichever call site actually exists, not by assuming one).

### 2. Acceptance Criteria
1. `emerald-cli examples/hello.em -o /tmp/out --verbose-cache` run
   twice prints all-MISS then all-HIT lines to stderr, and both runs'
   produced executables, run as subprocesses, print byte-identical
   stdout (`42\n`) — this plan's "zero behavior change, only speed"
   claim proven at the real CLI boundary a user actually invokes, not
   only against `emerald-driver`'s internal API.
2. `emerald build --verbose-cache` (plan 46), run twice with no source
   edits against its own two-package worked example, reports HIT for
   every query on the second run and does not re-invoke
   `emerald_codegen::compile_to_object` — closing the exact gap plan
   46's own worked example's comment left open (`# ... -> resolution
   skipped, only recompiled`).
3. Feeding the same two-line transcript to `emerald repl --history-
   cache /tmp/replay` twice: the first pass reports MISS and compiles
   normally; the second pass reports HIT for every previously-seen
   line and produces the identical printed output for each — the
   disclosed transcript-replay REPL win, proven end to end.
4. Regression: invoking `emerald-cli`/`emerald build`/`emerald run`/
   `emerald repl` with no cache-related flag at all behaves identically
   to their pre-this-plan selves — every existing test for plans 6, 46,
   and 47's own leaves (wherever already executed) keeps passing
   unmodified, proving `--verbose-cache`/`--history-cache` are strictly
   additive, opt-in flags.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs` (flag parsing, verbose
  reporting plumbed through the pipeline call), plan 46's build/run
  subcommand module (wherever it lands — `crates/emerald-cli/src/
  build.rs` per plan 46's own file list), plan 47's `crates/emerald-
  cli/src/repl.rs` (per plan 47's own file list)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| CLI end-to-end | `cargo test -p emerald-cli` | hello.em double-run HIT/MISS + identical stdout | agent-claimed-locally |
| Build/run integration | plan 46's own test harness, extended | second `emerald build` reports all-HIT, skips LLVM | agent-claimed-locally |
| REPL integration | plan 47's own test harness, extended | transcript replay reports HIT on the second pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
