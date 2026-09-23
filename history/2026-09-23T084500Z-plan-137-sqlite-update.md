2026-09-23T08:45:00Z

# Plan 137 — SQLite — Update

Update record for `history/2026-09-21T204600Z-plan-137-sqlite.md` (the
original plan text) — kept as a separate, dated file per this batch's
own append-only convention rather than editing the original. Read that
file first for the full original design (including its own Decision
log and Concrete Proof); this file records what was actually
implemented and confirms it matches the plan's own literal text with
no divergence.

## Status: implemented, all six leaves done

```
leaf-vet-rusqlite-vs-turso:     done
leaf-connection-handle:         done
leaf-prepare-and-bind:          done
leaf-execute-and-cursor:        done
leaf-error-mapping:             done
leaf-example-and-tests:         done
```

## No divergence from the plan's own literal text

Unlike several sibling plans in this batch, this one shipped exactly
as designed — the plan's own Concrete Proof compiles and runs
byte-for-byte as written (no `puts`/`while`/`Option[T]` adaptation was
needed), and every signature named in the plan's own todos list
(`Sqlite.open`/`.open_memory`/`.close`/`.execute_direct`/`.prepare`/
`.bind_string`/`.bind_int64`/`.bind_float64`/`.bind_null`/`.execute`/
`.query`/`.step`/`.column_string`/`.column_int64`/`.column_float64`/
`.begin`/`.commit`/`.rollback`) exists with exactly the parameter/
return shape the plan's own prose describes. The one syntax
adaptation the example needed was already anticipated by this
project's own established convention, not a plan-137-specific
surprise: `while <cond> do ... end`, not a bare `while <cond> ... end`
— confirmed directly against `bitwise_and_assignment.em`/`collections.
em`/`control_flow.em`/`domain_types.em`/`zip_roundtrip.em` before
writing `examples/sqlite_todo.em`, and applied verbatim.

## A real, disclosed design decision beyond the plan's own literal text
## — `Sqlite.close`'s live-child tracking, made concrete

The plan's own text says `Sqlite.close` errors "if any statement or
cursor handle still references it" without specifying the actual
mechanism. This implementation makes that concrete via a real
`open_children: i64` counter on `SqliteConnection` itself (`sqlite.rs`'s
own module doc has the full account): `Sqlite.prepare` increments it;
the *first* `Sqlite.execute`/`.query` call against that statement
decrements it back (a statement holds no live `rusqlite::Statement`
either way, so it stops being what blocks `close` the moment it first
runs, even though the handle itself stays valid and re-runnable);
`Sqlite.query`'s own returned cursor increments it again, decremented
only once `Sqlite.step` has walked past the cursor's last buffered
row. The Concrete Proof's own `insert`/`select` statements and `cursor`
are never explicitly closed (no such function exists in this surface),
so this counter genuinely has to return to zero through ordinary
`execute`/`query`/`step`-to-exhaustion alone for `Sqlite.close(db)` to
succeed at the end — verified directly by the passing example, not
merely designed to look correct. Matching `handle_close`'s own
established convention (`handle.rs`'s doc comment), closing an
already-closed or unknown connection is a harmless no-op, never a
raise — only a genuinely live connection with live children refuses to
close.

## Implementation

`crates/emerald-rt/src/sqlite.rs` (new module, ~500 lines excluding
tests): three `crate::handle`-registry type tags sharing one registry
(`SqliteConnection`/`SqliteStatement`/`SqliteCursor` — no `Type::
Newtype` for any of them; every handle crosses the FFI boundary as a
bare `i64`, exactly as the plan's own Concrete Proof types every
`Let`-bound handle `Int64`, never a domain-specific type name).
`SqliteValue` (an owned `Null`/`Integer`/`Real`/`Text`/`Blob` enum)
implements `rusqlite::types::ToSql` for the bind-parameter direction
and `From<ValueRef<'_>>` for the column-read direction — never a
borrowed value surviving past the one Rust function call that produced
it, the real mechanism that avoids `rusqlite::Statement<'conn>`'s
self-referential lifetime problem the plan's own Decision log names.
`Sqlite.prepare` never calls `conn.prepare` — only `sqlite_execute`/
`sqlite_query` ever materialize a real `rusqlite::Statement`, confirmed
directly against the plan's own Decision log wording before writing
the code. `BLOB` columns are stored (`SqliteValue::Blob`) but every
`column_*` accessor raises a `NativeError` naming the column if called
against one, matching the plan's own explicit v1 scope limit. 5
`#[test]`s (the Concrete Proof's own shape end-to-end; `bind_null`/
`bind_float64`/multi-row `ORDER BY` round-tripping; `.execute`'s real
affected-row count across an `INSERT ... VALUES (...), (...), (...)`
and an `UPDATE`; double-close as a harmless no-op; `.begin`/`.commit`/
`.rollback` actually undoing and actually keeping a real insert) — no
test exercises a `raise_native_error` path, deliberately: this crate's
own `#[cfg(test)]` `emerald_raise` stub (`lib.rs`) calls `std::process::
abort()`, the identical constraint `regex.rs`'s/`tempfile.rs`'s own
test suites already work within; the error-raising paths are proven
the same way every other plan-91/92 error path in this codebase is —
by the real `.em` example run through the real CLI, which never
exercises a failure branch here (this plan's own Concrete Proof has no
negative case), consistent with the plan's own text.

`crates/emerald-rt/Cargo.toml`/`DEPENDENCIES.md`: `rusqlite = {
version = "0.40", features = ["bundled"] }`, with the `turso`-vs-
`rusqlite` maturity comparison from the plan's own Decision log
reproduced in both the dependency's own doc comment and the ledger
row, per `leaf-vet-rusqlite-vs-turso`.

`crates/emerald-rt/src/lib.rs`: `mod sqlite;` plus 18 new `#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_*` wrappers, each
`catch_and_raise`-wrapped per plan 91's mandate — the mandatory
`std::panic::catch_unwind` boundary `leaf-error-mapping` calls for
lives here, at the `emerald_rt_*` export layer, the same place every
sibling domain module in this crate (`regex.rs`, `tempfile.rs`, etc.)
already puts it, not duplicated inside `sqlite.rs` itself.

`crates/emerald-sema/src/lib.rs`: `Sqlite` registered as a single
reserved-namespace static-call arm (the same shape `Xml`/`Env`/`Regex.
compile` already use) covering all 18 methods — no `Type::Newtype`
registration, no `ClassInfo` entry, and therefore no separate
instance-method-on-newtype-receiver dispatch arm anywhere else in this
file, a genuine simplification `Regex`/`XmlReader`/`Tempfile` do not
get to make.

`crates/emerald-codegen/src/lib.rs`: 18 new `Ctx` fields, 18 new
`module.add_function` declarations, one new static-call dispatch block
(`recv_name == "Sqlite"`) that branches once on whether `method` is one
of the nine `Void`-returning calls (built and returned without ever
calling `call_result` on a void LLVM call, the same convention `LogFields
#set`/`Regex`'s own sibling arms already establish) or one of the nine
value-returning calls, with `.step` narrowing its `i64` (0/1) ABI value
back into a real `i1` at the call site, the identical "widen the other
direction" trick `Regex#is_match` already establishes. **No newtype
registry entry was needed in either `NEWTYPE_UNDERLYING` or the
separate `newtypes.insert(...)` list near `compile_to_object_impl`** —
checked directly against the gotcha that bit plans 124/130/133,
confirmed inapplicable here specifically because this plan's own
Decision log deliberately types every handle as a bare `Int64`, never
a `Type::Newtype`, so neither registry's `Let`-binding-storage-kind
concern ever applies to `Sqlite`'s own handles in the first place.

`examples/sqlite_todo.em` / `crates/emerald-cli/tests/examples.rs`:
the plan's own Concrete Proof verbatim (only the `while ... do`
addition noted above), checked into the examples test table.

## A live, heavily contended shared working tree — how this plan's own
## edits were actually landed

This plan's own EXECUTE ran while at least one other agent (plan 150,
Glob) was concurrently, uncommittedly editing the exact same five
shared files this plan also needed (`crates/emerald-codegen/src/lib.
rs`, `crates/emerald-sema/src/lib.rs`, `crates/emerald-rt/src/lib.rs`,
`crates/emerald-rt/Cargo.toml`, `crates/emerald-rt/DEPENDENCIES.md`,
`crates/emerald-cli/tests/examples.rs`) — visible directly via `git
status --porcelain` showing `crates/emerald-rt/src/glob.rs`/`examples/
path_globbing_proof.em` as foreign untracked files throughout, and
later via several of the shared files showing `MM` (both staged AND
unstaged) status, meaning another agent had run its own `git add`
against them mid-session. This plan's own edits to those six shared
files were applied as small, surgical, anchor-based substitutions
(via exact-match `str.count() == 1` assertions, never a blind
line-range overwrite) chosen to sit at a different point in each file
than `Glob`'s own additions, specifically to avoid the two agents'
edits colliding at the same insertion point.

The commit itself used the `git commit-tree` + private `GIT_INDEX_FILE`
+ atomic `git update-ref` compare-and-swap technique plan 133's own
update doc first worked out under near-identical contention, applied
proactively here rather than reactively: for the six shared source
files, this plan's own intended final content was reconstructed by
applying this plan's own exact same substitutions directly to each
file's `git show <HEAD>:<path>` content (i.e. the last known-clean
commit, not the live working tree, which had `Glob`'s own uncommitted
hunks mixed in) — never by snapshotting the live working-tree files
directly, which would have silently swept `Glob`'s own in-progress,
not-yet-reviewed code into this commit under this plan's own message,
the exact mistake plan 133's own agent made and had to reverse out of
twice. `Cargo.lock` was the one deliberate exception to that rule: a
`cargo`-regenerated, non-hand-authored file where `Glob`'s own
transitive dependency additions and this plan's own `rusqlite`/
`libsqlite3-sys`/etc. additions are purely additive and mutually
non-conflicting at the file-content level (unlike the two plans'
actual feature code), so the live working tree's already-merged
`Cargo.lock` was used as-is rather than hand-reconstructed — a
correctly resolved lockfile for the full combined dependency graph is
the only thing this specific file is at all. Every new file this plan
adds (`crates/emerald-rt/src/sqlite.rs`, `examples/sqlite_todo.em`,
this update doc) came from the live working tree directly, since
nothing else could have written to those paths. The resulting commit
was verified by diffing the actual committed tree content (`git show
<commit>:<path>`) against the intended reconstructed content for every
touched shared file — not merely a `--stat` line-count check — before
this task was reported done.

## Gate

`cargo build -p emerald-rt -p emerald-sema -p emerald-codegen`: clean.
`cargo test -p emerald-rt sqlite::` (isolated, `--test-threads` default
— five tests, all pure in-memory, no shared-resource flakiness
possible): 5/5 passed. `cargo build --workspace`: clean. `cargo test -p
emerald-cli --test examples sqlite_todo -- --test-threads=1`: 1/1
passed, exact expected stdout (`"1\nwrite plan 137\n"`). `cargo clippy
-p emerald-rt -p emerald-sema -p emerald-codegen --all-targets`: 0
warnings/errors attributable to `sqlite.rs` or this plan's own edits
(grepped directly for `sqlite` in the full clippy output — zero
matches); the pre-existing `missing_safety_doc` warnings shared by
every sibling `unsafe extern "C" fn` in this crate, and one pre-
existing `needless_return` warning in plan 133's own unrelated `Zip`
arm, are untouched, not this plan's. `treefmt`: reformatted this
plan's own new/edited Rust files (whitespace-only); re-verified after.

## Explicitly out of scope (unchanged from the original plan)

Everything the original plan's own "Out of scope" section already
named: connection pooling, a query builder/ORM, SQLite extensions/
virtual tables/FTS5, and a true incremental (non-buffering) result
cursor.
