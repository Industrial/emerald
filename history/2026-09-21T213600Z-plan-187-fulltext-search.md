2026-09-21T21:36:00Z

---
name: Full-Text Search — Embedding `tantivy` as a First-Class Standard-Library Capability
overview: "A new `emerald-rt` module wrapping `tantivy` 0.26.x — a real, fast, mmap-backed, BM25-scoring full-text search engine library, Lucene's closest pure-Rust analog — as `SearchSchema`/`SearchIndex`/`SearchWriter`/`SearchResults` resource types, giving Emerald programs a real embedded search engine with no external service (no Elasticsearch, no Solr, no separate process) required. This is one of the standout, genuinely exciting capabilities in this entire 101-plan batch: most languages need to shell out to a running search server for this; an Emerald program links it directly into its own binary."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-search-module-and-schema-resource
    content: "Create `crates/emerald-rt/src/search.rs`. Register four compiler-provided, non-user-declarable classes (each one hidden `u64` handle field, per plan 93): `SearchSchema` (→ `tantivy::schema::Schema`, built via `SchemaBuilder`), `SearchIndex` (→ `tantivy::Index`), `SearchWriter` (→ `tantivy::IndexWriter`), and `SearchResults` (→ a `Vec<(f32, tantivy::schema::TantivyDocument)>`, the collected, already-scored, already-fetched hit list — see `leaf-query-and-search`). Every exported `emerald_rt_search_*` function goes through plan 92's `emerald_rt_fn!` catch-unwind macro; a `tantivy::TantivyError` (a corrupt index directory, a malformed query string, a schema mismatch) converts to a `NativeError` at the point it surfaces, or to `Result[T, String]` where the failure is a routine, expected one (see `leaf-query-and-search`'s query-parse-error handling)."
    status: pending
  - id: leaf-schema-builder
    content: "`SearchSchema.new(): SearchSchema` (`Schema::builder()`), `.add_text_field(name: String, stored: Boolean): Void` (`TEXT | STORED` when `stored` is `true`, bare `TEXT` — tokenized and indexed with term frequency and positions, per `tantivy`'s own real, current field-flag semantics verified against its published example this session — otherwise), `.add_u64_field(name: String, stored: Boolean, indexed: Boolean): Void` (a numeric fast field, for range queries/sorting — `tantivy`'s own real distinct field kind alongside text, not folded into the text path), and `.build(self): Void` (freezes the schema — after this call, no more fields may be added; `tantivy`'s own `Schema` is itself immutable once built, so this is a real, disclosed one-way state transition on the handle, not an arbitrary restriction this plan invents)."
    status: pending
  - id: leaf-index-and-writer
    content: "`SearchIndex.create_in_dir(path: String, schema: SearchSchema): SearchIndex` (`Index::create_in_dir(&path, schema)` — a real, persistent, on-disk index directory, `tantivy`'s own `meta.json`-plus-segment-files layout, not an in-memory-only structure; `Index::create_in_ram(schema)` is also exposed as `SearchIndex.create_in_ram(schema: SearchSchema): SearchIndex` for a caller that wants a scratch/test-only index with no filesystem footprint at all). `SearchIndex.writer(self, heap_bytes: Int64): SearchWriter` (`.writer(heap_bytes as usize)` — `tantivy`'s own documented minimum indexing memory budget applies unmodified; a caller supplying too small a value gets `tantivy`'s own real error, converted to `NativeError`, not a silently-adjusted-upward value this plan would otherwise have to justify). `SearchWriter.add_text_document(self, field_values: Array[String], field_names: Array[String], count: Int64): Void` (builds one `TantivyDocument` from the given parallel `(name, value)` arrays — both `Array[String]`, both needing an explicit `count` for the identical no-length-metadata reason plan 182's own `ARGV`/`ARGC` marshaling leaf documents; `Array[T]` carries no length prefix per plan 45's own verified finding — and calls `.add_document(doc)`), and `.commit(self): Void` (`.commit()` — makes newly added documents visible to new searches; `tantivy`'s own real two-phase add-then-commit model, not this plan's invention, disclosed explicitly since a caller who forgets to `.commit()` will see an index that silently still returns zero results for a just-added document)."
    status: pending
  - id: leaf-query-and-search
    content: "`SearchIndex.search(self, query_str: String, default_fields: Array[String], field_count: Int64, limit: Int64): Result[SearchResults, String]` — internally: `index.reader()?` (a `tantivy::IndexReader`, using the crate's own `ReloadPolicy::OnCommitWithDelay` so a search sees documents committed by a prior `SearchWriter.commit()` call without the caller managing reader-reload timing by hand), `QueryParser::for_index(&index, default_fields).parse_query(query_str)` — a malformed query string (unbalanced parentheses, an unknown field name) is `tantivy`'s own real, routine `QueryParserError`, converted to `Err(message)` via plan 92's `emerald_rt_result_err`, not a `NativeError`, since a bad user-typed search string is exactly the kind of expected, anticipatable failure plan 92/182/185/186 already draw this same line for — then `searcher.search(&query, &TopDocs::with_limit(limit as usize))`, fetching each hit's stored fields via `searcher.doc(doc_address)`. `SearchResults` accessors: `.count(self): Int64`, `.score(self, i: Int64): Float64` (the real BM25 score `tantivy` computed, not a placeholder), `.field_value(self, i: Int64, field_name: String): Option[String]` (the stored value for that field on that hit, `None` if the field wasn't marked `STORED` at schema-build time or wasn't present on that document)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/fulltext_search.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs`'s CI-checked table per plan 95's checklist, run from a fresh temporary directory (so the on-disk index directory is created, not pre-existing, matching plan 45's own `File.write`/`File.read` temp-dir precedent). Add `#[test]`s in `emerald-rt` covering: schema build, document indexing and commit, a query matching exactly the expected documents by content, a query matching zero documents, a malformed query string producing `Err` rather than a panic, and a stored-field readback matching the originally indexed value exactly. Run the full `AGENTS.md` gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`) plus a clean-checkout end-to-end build."
    status: pending
isProject: false
---

# Plan 187 — Full-Text Search (`tantivy`)

Every other plan in this 101-plan batch wraps something a program
either already had a manual, worse alternative to (hand-parsed CLI
flags), or that most languages already ship a standard-library-adjacent
version of (JSON, CSV, compression). Full-text search is different: in
most languages, "search these million documents for the best matches to
this query, ranked by relevance" means standing up Elasticsearch or
Solr — a whole separate service, its own deployment, its own network
hop, its own operational surface — because no mainstream language ships
a real search engine in-process. `tantivy` is the reason Emerald doesn't
have to make that trade. It is a genuine, fast, mmap-backed, BM25-
scoring, Lucene-inspired search engine library, not a toy substring
matcher — the same engine Quickwit (a real distributed search platform)
builds on top of, used here as a plain, embeddable, no-external-process
dependency. This plan is explicitly one of the more exciting
capabilities in the whole batch: an Emerald program gets Lucene-class
search, linked directly into its own binary, with `puts`-simple call
sites on top.

Depends on: plan 91 (`emerald-rt` archive), plan 92 (`catch_unwind`
convention, `NativeError`, `Result[T,E]` helpers), plan 93 (opaque-
handle resource model for all four resource types this plan defines),
plan 95 (crate-vetting policy and checklist), and plan 45 (`Array[T]`'s
own no-length-metadata representation, the reason `SearchWriter.add_
text_document`/`SearchIndex.search` both need an explicit count
alongside every `Array[String]` argument, exactly as plan 182 already
established for `ARGV`/`ARGC`).

## Concrete proof this plan targets

Run from a fresh temporary working directory:

```ruby
schema: SearchSchema = SearchSchema.new()
schema.add_text_field("title", true)
schema.add_text_field("body", false)
schema.build()

index: SearchIndex = SearchIndex.create_in_dir("search_demo_index", schema)
writer: SearchWriter = index.writer(50000000)

fields1: Array[String] = ["title", "body"]
values1: Array[String] = ["The Old Man and the Sea", "He was an old man who fished alone in a skiff."]
writer.add_text_document(values1, fields1, 2)

fields2: Array[String] = ["title", "body"]
values2: Array[String] = ["Of Mice and Men", "A few miles south of Soledad, the river drops in close to the hillside."]
writer.add_text_document(values2, fields2, 2)

writer.commit()

query_fields: Array[String] = ["title", "body"]
result: Result[SearchResults, String] = index.search("old man fished", query_fields, 2, 10)

case result
when Ok(hits)
  puts hits.count()
  title: Option[String] = hits.field_value(0, "title")
  case title
  when Some(t)
    puts t
  when None
    puts "no title"
  end
when Err(msg)
  puts msg
end
```

Expected output: `1` (exactly one of the two indexed documents contains
"old man"/"fished"), then `The Old Man and the Sea` (the stored
`title` field of the single matching hit, retrieved through
`tantivy`'s own BM25-ranked top-doc collector) — a real, working,
end-to-end proof that schema definition, on-disk indexing, commit
visibility, query parsing, and stored-field retrieval all function
together, not a mocked or hand-simulated search.

## Decision log

- **`tantivy`, verified current, real, and Lucene-adjacent this
  session — not a toy or abandoned crate.** Version 0.26.2, published
  8 September 2026 (two weeks before this plan), maintained under the
  Quickwit organization (`fulmicoton`/`PSeitz`) — its own README states
  its purpose plainly: "closer to Apache Lucene than to Elasticsearch or
  Apache Solr... a crate that can be used to build such a search
  engine," explicitly citing BM25 scoring "the same as Lucene," phrase
  queries, faceted search, incremental multithreaded indexing, and an
  mmap-backed directory format — all real, documented, current
  capabilities, not aspirational ones this plan is assuming into
  existence.
- **The Emerald-facing API mirrors `tantivy`'s own documented example
  program directly — verified against its published `basic_search.rs`
  this session, not invented from a guess at the crate's shape.** The
  real `tantivy` flow is: build a `Schema` via `SchemaBuilder` with
  `TEXT | STORED`-style field flags, `Index::create_in_dir`, obtain an
  `IndexWriter` with an explicit memory budget (`50_000_000` bytes is
  `tantivy`'s own example's chosen figure, reused here as this plan's
  own default), add documents, `.commit()`, then query via
  `QueryParser`/`TopDocs`. This plan's `SearchSchema`/`SearchIndex`/
  `SearchWriter`/`SearchResults` sequence is a direct, verified
  transliteration of that exact real flow into Emerald's own resource-
  handle idiom, not a redesign.
- **`TEXT`/`STORED` field flags collapse to one `Boolean` parameter in
  v1, not `tantivy`'s own full flag algebra.** `tantivy`'s real
  `add_text_field` takes a composable `TextOptions` value (`TEXT`,
  `STORED`, `TEXT | STORED`, plus finer-grained tokenizer/indexing-
  record-option control this plan does not expose at all). This plan's
  `.add_text_field(name, stored)` always requests full tokenized,
  indexed-with-positions text search (`TEXT`, unconditionally), and
  `stored` toggles only whether the original value round-trips back out
  through `.field_value()` — the two knobs every real caller actually
  needs for a basic search feature, without exposing `tantivy`'s much
  larger configurable-tokenizer surface (stemming-language selection,
  custom analyzers) as v1 scope.
- **A commit is required and visible-after-delay, not synchronous —
  `tantivy`'s own real model, stated rather than hidden.** `tantivy`'s
  indexing is asynchronous by design (multithreaded background merge/
  commit machinery); this plan's `IndexReader` is built with
  `ReloadPolicy::OnCommitWithDelay` specifically so a search issued
  shortly after `.commit()` returns sees the new documents without the
  caller manually polling or reopening the index — but "shortly after,"
  not "instantaneously synchronous with," is a real, disclosed
  characteristic of the underlying engine this plan does not paper
  over. The worked proof above works because the example runs `.search`
  as a separate, later call after `.commit()` returns, giving the
  reload policy room to apply — a program calling `.search` in a tight
  loop immediately after `.commit()` with no intervening work should
  not assume synchronous visibility.
- **Query syntax is `tantivy`'s own real query language, unmodified —
  this plan adds no query-parsing logic of its own.** `tantivy`'s
  `QueryParser` already supports boolean `AND`/`OR`, field-scoped terms
  (`title:sea`), phrase queries (`"old man"`), and more; this plan's
  `SearchIndex.search` hands the caller's `query_str` straight to
  `QueryParser::parse_query` with zero preprocessing, so the full real
  query language is available to Emerald callers immediately, not a
  reduced subset this plan would otherwise have to design and document
  separately.
- **A malformed query is `Result`, not `NativeError` — the same
  expected-failure line plan 92/182/185/186 already draw, applied here
  to user-typed search strings.** A caller building a search feature on
  top of this module very likely exposes a raw search box to an end
  user at some point, and an end user typing an unbalanced quote or an
  unknown field name is routine, anticipated input — not a Rust-side
  bug. `Err(String)` carrying `tantivy`'s own formatted parser error
  keeps that a normal `case`/`when` branch.
- **Out of scope.** Faceted search, aggregation collectors (histogram/
  range-bucket/stats), and custom tokenizers/stemmers — all real,
  documented `tantivy` features (its own README lists all three) this
  plan's v1 does not expose; the basic `TEXT | STORED`-versus-numeric-
  fast-field schema and `QueryParser`/`TopDocs` search path is the
  whole surface. Distributed/sharded search across multiple index
  directories or machines — `tantivy`'s own README states plainly that
  distributed search is explicitly out of its scope too, pointing
  instead at Quickwit; this plan inherits that same boundary rather
  than attempting to build distribution on top. Index schema migration
  (changing a field's type/flags on an existing on-disk index) — not a
  `tantivy` feature at all; a schema change means building a new index.
  Fuzzy/typo-tolerant query support and custom scoring functions beyond
  `tantivy`'s own default BM25 — real, separate, more advanced `tantivy`
  features not wired into this plan's v1 `QueryParser`-only path.
