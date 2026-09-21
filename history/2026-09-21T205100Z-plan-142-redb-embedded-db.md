2026-09-21T20:51:00Z

---
name: Embedded ACID Database (redb) — This Batch's Recommended Embedded Key-Value Store
overview: "A `Redb` compiler-provided namespace wrapping `redb` — a pure-Rust, ACID-transactional embedded key-value store checked this session to be actively maintained (a stable release six days before this plan's authoring, 38 stable releases total, steadily growing adoption), positioned explicitly as the batch's recommended default over plan 141's stalled `sled`. Covers real begin/commit/abort write transactions and concurrent read transactions against named, runtime-interned tables, and states plainly when an Emerald program should reach for this over plan 137's SQLite: fixed-shape exact-key lookups with no query language, versus anything needing filtering, joins, or ad hoc reporting."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vet-redb-vs-sled
    content: "Record the comparative vetting decision in `emerald-rt/Cargo.toml`'s doc comment above `redb = \"4\"`: verified this session, `redb` (crates.io `4.3.0`, released 2026-09-15, 38 stable releases, MIT/Apache, 1.9M downloads/month and still climbing week over week, `cberner/redb`, 40 contributors) is actively maintained where plan 141's `sled` has not published a release since 2024-10-11 — this plan is the recommended default for new Emerald code needing an embedded KV store, cross-referenced from plan 141's own Decision log."
    status: pending
  - id: leaf-connection-handle
    content: "`Redb.open(path: String): Int64` — `redb::Database::create(path)` (creates if absent, opens if present — redb's own documented behavior), registered into a `Mutex<HashMap<u64, redb::Database>>` per plan 93's opaque-handle convention. `Redb.close(db: Int64): Void` removes and drops the entry."
    status: pending
  - id: leaf-table-handles
    content: "`Redb.table(name: String): Int64` returns a table-schema handle, not tied to any one `Database` — internally, a global `Mutex<HashMap<String, &'static str>>` interns each distinct `name` exactly once (via a single `Box::leak` per never-before-seen name, never per call) to satisfy `redb::TableDefinition<'static, &str, &str>`'s real `'static`-lifetime requirement for a runtime-supplied table name, then caches the constructed `TableDefinition` behind an `Int64` handle in a second registry keyed by that same interned name."
    status: pending
  - id: leaf-transactions
    content: "`Redb.begin_write(db: Int64): Int64` (`Database::begin_write`) and `Redb.begin_read(db: Int64): Int64` (`Database::begin_read`) return distinct write- and read-transaction handles in separate registries (mirroring plan 141's pattern for pubsub-vs-command connections in plan 140). `Redb.table_insert/table_get/table_remove(txn: Int64, table: Int64, key: String, value: String?): String?` open the given table within the given transaction (`WriteTransaction::open_table`/`ReadTransaction::open_table`) and perform the operation — `table_get` is valid against either transaction kind (redb write transactions can read their own uncommitted writes), `table_insert`/`table_remove` are plan-93 errors if the handle names a read transaction. `Redb.commit(txn: Int64): Void` consumes and commits a write transaction; `Redb.abort(txn: Int64): Void` rolls it back explicitly (re-verify redb's exact abort-vs-drop-without-commit semantics against its real, current API at execution time, the same discipline plan 54 applied to an assumed dependency's contract)."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/redb_kv.em` (the Concrete Proof below) to `examples/` and the `emerald-cli/tests/examples.rs` table. Add `#[test]`s in `emerald-rt` against a `tempfile::tempdir()`-backed `Database::create` — no external process or `testcontainers` fixture needed, matching plan 137/141's in-process posture; explicitly test that an aborted write transaction's changes are not visible to a subsequent read transaction, proving the transactional boundary is real, not merely present in the API surface."
    status: pending
isProject: false
---

# Plan 142 — Embedded ACID Database (redb)

Where plan 141 had to report an uncomfortable finding about `sled`, this
plan gets to report a comfortable one about `redb`: checked directly
this session, `redb` published `4.3.0` six days before this plan's
authoring date, its 38th stable release, with download volume (1.9M/
month) that has grown every week for the last several months rather than
plateaued. `redb` is a from-scratch, pure-Rust, ACID-transactional
embedded key-value store — B-tree-based, MVCC (multiple concurrent
readers alongside one writer), typed tables declared via `Table
Definition`. This plan positions `redb` as the batch's recommended
default for "I need an embedded, durable, exact-key-lookup store" —
plan 141's `sled` exists for reading data that already exists in that
format, not as a competing recommendation.

`redb`'s transaction model is also a structurally better fit for this
project's FFI calling convention than `sled`'s, independent of the
maintenance question. `sled`'s transactions are Rust-closure-based
(`Db::transaction(|tx| { ... })`), which plan 141's Decision log
explains cannot cross this project's `extern "C"` boundary without a
reentrant-callback marshaling mechanism this batch does not build.
`redb`'s transactions are explicit, first-class values —
`begin_write()`/`begin_read()` return a transaction object a caller
holds, mutates through, and later calls `.commit()` or lets roll back —
exactly the request/response, no-callback shape every other handle in
this batch (a SQL connection, a Redis connection, a prepared statement)
already has. `redb`'s design needs no FFI accommodation `sled`'s would
have required from scratch.

## Concrete proof this plan targets

```ruby
db: Int64 = Redb.open("plan142_demo.redb")
users: Int64 = Redb.table("users")

write_txn: Int64 = Redb.begin_write(db)
Redb.table_insert(write_txn, users, "1", "Ada")
Redb.table_insert(write_txn, users, "2", "Grace")
Redb.commit(write_txn)

read_txn: Int64 = Redb.begin_read(db)
first: String? = Redb.table_get(read_txn, users, "1")
first ||= "unknown"
puts first

second: String? = Redb.table_get(read_txn, users, "2")
second ||= "unknown"
puts second

missing: String? = Redb.table_get(read_txn, users, "3")
missing ||= "not found"
puts missing

Redb.close(db)
```

Expected output: `Ada`, `Grace`, `not found` — two rows written and
committed in one write transaction, then read back through a separate
read transaction, with a genuine miss defaulted via plan 43's `||=`.

## Decision log

- **`redb`'s maintenance signal is the direct, favorable counterpoint to
  plan 141's — both checked with the same rigor, not asserted by
  reputation.** A release six days old at authoring time, a steady
  38-release stable cadence, and climbing (not merely high) download
  volume are all real signals of an actively developed project, the
  same category of evidence plan 141 used to reach the opposite
  conclusion about `sled`. This plan does not claim `redb` is superior
  in the abstract — it claims `redb` is *currently maintained* and
  `sled` currently is not, which is the concrete, checkable question
  the task brief asked to be answered honestly.
- **Transactions are the real, explicit `begin_write`/`open_table`/
  `commit` sequence, not a hand-waved "it has transactions."** A write
  transaction (`Database::begin_write`) opens one or more tables within
  itself, accumulates inserts/removes against them, and only becomes
  durable and visible to other transactions at `.commit()` — an
  uncommitted write transaction's changes are invisible to any
  concurrently open read transaction, and this plan's own `#[test]`
  suite (`leaf-example-and-tests`) specifically asserts that an aborted
  write transaction's changes never appear in a later read, the concrete
  proof that this is real MVCC isolation and not merely an API that
  accepts a `commit()` call without enforcing anything.
- **Table names need runtime interning to satisfy a real Rust API
  constraint — `redb::TableDefinition<'static, K, V>` requires a
  `'static` name, but Emerald table names only exist as runtime
  `String` values.** The resolution is a one-time-per-distinct-name
  `Box::leak`, cached in a global interner keyed by the name string so
  a program calling `Redb.table("users")` a thousand times leaks the
  string exactly once, not a thousand times — a bounded, disclosed
  amount of intentional leaking (proportional to the number of distinct
  table names a program actually declares, typically a handful, never
  proportional to the number of rows or operations), the same category
  of accepted tradeoff `emerald_alloc`'s own "never free" default
  already established as precedent in this codebase (plan 08).
- **`redb` vs. plan 137's SQLite: a real, stated distinction, not "pick
  whichever."** `redb` has no query language, no secondary indexes, no
  `WHERE`-clause filtering, no joins — every access is an exact key
  lookup against a table whose shape is fixed by the program itself, not
  discoverable or filterable at query time. Reach for `Redb` when the
  access pattern is genuinely "get/put by a key I already know" — a
  session store, a durable cache, a config or feature-flag table, an
  index keyed by an ID a caller already has. Reach for `Sqlite`/
  `Postgres`/`Mysql` the moment a program needs to ask "which rows have
  property X" without already knowing which keys those are — that
  question has no honest answer in `redb`'s data model without the
  caller building and maintaining their own secondary index by hand.
- **`redb` vs. plan 141's `sled`: recommend `redb`, for two independent
  reasons, both stated rather than implied.** Maintenance currency (see
  above) is one; the other is that `redb`'s explicit transaction-object
  API needs no FFI accommodation this batch would otherwise have to
  invent from scratch for `sled`'s closure-based one (see the plan
  introduction above). A program with no existing `sled`-format data has
  no real reason to choose plan 141 over this plan.
- **Values marshal as `String`, not raw bytes — the same disclosed
  `Bytes`-type gap every other storage plan in this batch (137, 138,
  139, 140, 141) has already hit.** `redb` genuinely supports arbitrary
  `&[u8]` keys/values at the Rust level; this plan's Emerald-facing
  surface narrows both to `String` for the same reason plan 141 did —
  no byte-buffer type exists in Emerald's real `Type` enum (plan 59) —
  with the identical disclosed UTF-8-validity requirement on write and
  UTF-8-validity check on read.
- **Out of scope.** Multiple physical database files/sharding across
  them, savepoints and multi-version snapshot reads beyond the basic
  read/write transaction pair described above, `redb`'s `no_std` support
  (real — `redb` genuinely builds under `no_std` with `alloc`, per its
  own crates.io listing — but no Emerald target this batch addresses
  needs it), and any secondary-index or query-planning layer over
  `redb`'s tables — this plan is a direct key-value driver, not a
  database engine of its own.
