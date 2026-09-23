2026-09-23T07:04:00Z

# Plan 142 — Embedded ACID Database (redb) — Update

Update record for `history/2026-09-21T205100Z-plan-142-redb-embedded-
db.md` (the original plan text) — kept as a separate, dated file per
this batch's own append-only convention rather than editing the
original. Read that file first for the full original design (including
its own Decision log and Concrete Proof); this file records what was
actually implemented and the one real, disclosed syntax correction the
example needed.

## Status: implemented, all five leaves done

```
leaf-vet-redb-vs-sled:   done
leaf-connection-handle:  done
leaf-table-handles:      done
leaf-transactions:       done
leaf-example-and-tests:  done
```

## A real, disclosed correction against the plan's own literal
## Concrete Proof text — `String?`/`||=` are dead syntax

The plan's own Concrete Proof (authored 2026-09-21) types
`Redb.table_get`'s result as `String?` and unwraps a miss via
`first ||= "unknown"`. Both are dead syntax: plan 73 removed the `T?`
nullable-sugar annotation and the `||=` operator outright — the same
correction plan 146's own `environment_variables_proof.em` already had
to make for `Env.get`, and the same one `charset_encoding_proof.em`/
`extended_filesystem_proof.em` independently disclose for their own
plans. `examples/redb_kv.em` uses the real, current surface instead:
`Option[String]` as the type annotation, `match ... do Some(v) do ...
end None do ... end end` to unwrap, discovered directly by attempting
to compile the plan's own literal text first and reading the parser's
own diagnostic, not assumed in advance. Output is unchanged —
`"Ada\nGrace\nnot found\n"`, exactly the plan's own stated expectation.

## `redb`'s own API surface, re-verified directly against the real
## pinned crate rather than an older vendored copy

`ReadableDatabase`/`ReadableTable` are real, necessary trait imports
in `redb` 4.x — `Database::begin_read`/`Table::get`/`ReadOnlyTable::
get` are trait-provided, not inherent methods, on this pinned version
(confirmed the hard way: a first `cargo check` against an older locally
cached `redb` 3.1.0 source tree suggested `begin_read` was inherent;
the actual resolved 4.x dependency disagreed, and the fix was one
import). `WriteTransaction::commit(self)`/`.abort(self)` both consume
`self` by value on this pinned version too, matching the plan's own
Decision log's expectation and this implementation's `Option<redb::
WriteTransaction>`-wrapped-and-`.take()`n design.

## Implementation

`crates/emerald-rt/src/redb_kv.rs` (new module, ~360 lines excluding
tests): four `crate::handle`-registry type tags sharing one registry
(`RedbDatabase`/`RedbTable`/`RedbWriteTxn`/`RedbReadTxn` — no `Type::
Newtype` for any of them, the identical shape plan 137's `Sqlite`
already established, confirmed against this plan's own Decision log
before writing any code). `redb_table` interns each distinct table
name via one `Box::leak` per never-before-seen name, cached in a
`Mutex<HashMap<String, i64>>` keyed by the name itself so repeat calls
with the same name return the same handle (proven by its own
`#[test]`, not merely designed to). `table_get` tries the write-
transaction tag first, then the read-transaction tag — reusing
`handle_get_mut`'s own existing type-tag-mismatch error for
`table_insert`/`table_remove` against a read-transaction handle
(`leaf-transactions`'s own "plan-93 error" requirement) rather than
writing bespoke validation. 3 `#[test]`s: the Concrete Proof's own
round trip end to end; calling `.table` twice with the same name
returns the same handle; and, the plan's own specifically required
proof, that an aborted write transaction's changes are never visible
to a later read transaction (seeded via a separate committed write
first, since `ReadTransaction::open_table` itself raises if the table
was never created — a real, disclosed `redb` behavior, not an
oversight). A fourth test asserting `table_insert` through a read-
transaction handle raises was written, then removed: this crate's own
`#[cfg(test)]` `emerald_raise` stub calls `std::process::abort()`, not
a catchable panic (confirmed by hitting a `SIGABRT` directly, isolated
down to a single `--test-threads=1` run of that one test alone before
concluding it was structural, not a leftover in this plan's own code)
— the identical constraint plan 137's own update doc already names for
`sqlite.rs`. That raise path is left to `examples/redb_kv.em`'s real,
full-pipeline CLI run instead, which never exercises it (no negative
case in the Concrete Proof), consistent with `sqlite.rs`'s own
precedent.

`crates/emerald-rt/Cargo.toml`/`DEPENDENCIES.md`: `redb = "4"`
(resolved to `4.3.0`), with the redb-vs-sled maintenance comparison
from the plan's own Decision log reproduced in both the dependency's
own doc comment and the ledger row, per `leaf-vet-redb-vs-sled`.

`crates/emerald-rt/src/lib.rs`: `mod redb_kv;` (named, not a bare `mod
redb;`, to avoid shadowing the external `redb` crate — the identical
collision `tempfile.rs`/`toml.rs`/`url.rs` already disclose) plus 10
new `#[no_mangle] pub unsafe extern "C" fn emerald_rt_redb_*` wrappers,
each `catch_and_raise`-wrapped per plan 91's mandate.

`crates/emerald-sema/src/lib.rs`: `Redb` registered as a single
reserved-namespace static-call arm (the same shape `Sqlite` above it
already uses) covering all 10 methods — no `Type::Newtype`
registration, no `ClassInfo` entry, so no separate instance-method-on-
newtype-receiver dispatch arm exists anywhere else in this file for
`Redb` either. `table_insert`/`table_get`/`table_remove` all return
`Type::Enum("Option$String".to_string())`, the same type `Env.get`
already returns.

`crates/emerald-codegen/src/lib.rs`: 10 new `Ctx` fields, 10 new
`module.add_function` declarations, one new static-call dispatch block
(`recv_name == "Redb"`) that branches first on whether `method` is one
of the three `Option[String]`-returning table operations (reusing the
exact `is_null`-branch-plus-`phi` construction `Env.get`'s own call
site already establishes, copied verbatim rather than re-derived) or
one of the seven plain `Int64`/`Void`-returning calls. **No newtype
registry entry was needed in either `NEWTYPE_UNDERLYING` or the
separate `newtypes.insert(...)` list near `compile_to_object_impl`** —
checked directly against the gotcha that bit plans 124/130/133,
confirmed inapplicable here for the same reason `sqlite.rs`'s own
update doc already gives: this plan's own Decision log deliberately
types every handle as a bare `Int64`, never a `Type::Newtype`.

`examples/redb_kv.em` / `crates/emerald-cli/tests/examples.rs`: the
plan's own Concrete Proof, adapted only for the `String?`/`||=`
correction above; checked into the examples test table.

## A live, heavily contended shared working tree — how this plan's
## own edits were actually landed

This plan's own EXECUTE ran while the shared working tree already
carried other agents' foreign, uncommitted edits to several of the
same shared files this plan also needed (`crates/emerald-codegen/src/
lib.rs`, `crates/emerald-sema/src/lib.rs`, `crates/emerald-rt/src/lib.
rs`, `crates/emerald-rt/Cargo.toml`, `crates/emerald-rt/DEPENDENCIES.
md`, `crates/emerald-cli/tests/examples.rs`), and `HEAD` itself moved
once during this plan's own research phase (`3fc5546`, plan 137, to
`60cbc80`, plan 150) — both confirmed directly via repeated `git status
--porcelain`/`git log --oneline -5`, not assumed stable. To avoid any
risk of a naive `git add`/`git commit` picking up a concurrent agent's
unreviewed in-progress hunks (the exact failure mode plan 133's own
update doc and plan 137's own update doc both document and both
worked around the same way), this plan's own changes were developed
and fully gated (`cargo build --workspace`, `cargo nextest run`,
`cargo clippy --all-targets`, `cargo fmt --check`, `cargo audit`) in a
fully isolated clone checked out at the real `60cbc80` `HEAD`, entirely
outside the shared working tree, before touching the shared repository
at all. The commit itself was then constructed via the `git commit-
tree` + private `GIT_INDEX_FILE` + atomic `git update-ref` compare-
and-swap technique: for every shared file, the intended final content
was the isolated clone's own edited copy (itself built by applying
exact-match, anchor-based substitutions directly against each file's
`git show <HEAD>:<path>` content, never against the live, contended
working-tree copy); `Cargo.lock` used the isolated clone's own
`cargo`-regenerated lockfile (a correct resolution of clean `HEAD`'s
existing dependency graph plus this plan's own single additive `redb`
entry — `redb` itself pulls in zero further transitive dependencies,
verified directly via `git diff Cargo.lock` showing exactly one new
`name = "redb"` block — itself a derived file with no live-working-
tree edits to preserve, since this plan's own isolated build never saw
any other agent's concurrent Cargo.toml edits). The two genuinely new
files (`crates/emerald-rt/src/redb_kv.rs`, `examples/redb_kv.em`, this
update doc) came from the isolated clone directly, since nothing else
could have written to those paths. The resulting commit was verified
by diffing the actual committed tree content (`git show <commit>:
<path>`) against the intended content for every touched file — not
merely a `--stat` line-count check — before this task was reported
done.

## Gate

`cargo check -p emerald-rt`: clean. `cargo test -p emerald-rt --lib
redb_kv -- --test-threads=1` (isolated, no shared-resource flakiness
possible — every test uses its own uniquely named `tempfile::
tempdir()`-backed `.redb` file): 3/3 passed. `cargo check -p
emerald-sema -p emerald-codegen`: clean. `cargo build --workspace`:
clean. Compiled and ran `examples/redb_kv.em` directly through the
real CLI: exact expected stdout (`"Ada\nGrace\nnot found\n"`). `cargo
nextest run -p emerald-rt -p emerald-sema -p emerald-codegen -p
emerald-cli --test-threads 4`: 940/940 passed (2 pre-existing skips,
unrelated), including all 3 new `redb_kv::tests::*` and the new
`redb_kv_em_prints_expected_sequence` example test. `cargo clippy -p
emerald-rt -p emerald-sema -p emerald-codegen --all-targets`: 0
warnings/errors attributable to `redb_kv.rs` or this plan's own edits
(grepped directly for `redb` in the full clippy output — zero
matches); the pre-existing `missing_safety_doc`/`Sqlite`-arm
formatting warnings shared by other, unrelated code in this crate are
untouched, not this plan's. `cargo fmt --check` on every touched
crate: clean after `cargo fmt -p emerald-rt` (touched only this
plan's own new `redb_kv.rs`, verified via `git status --porcelain` on
the isolated clone before and after); one pre-existing, unrelated
`Sqlite`-arm formatting diff already present at clean `HEAD` was left
untouched, not this plan's to fix. `cargo audit --ignore
RUSTSEC-2023-0071`: exit 0, no advisory against `redb` itself, only
the same pre-existing `unmaintained`/`unsound` `id_effect`-transitive
warnings `DEPENDENCIES.md` already triages.

## Explicitly out of scope (unchanged from the original plan)

Everything the original plan's own "Out of scope" section already
named: multiple physical database files/sharding, savepoints and
multi-version snapshot reads beyond the basic read/write transaction
pair, `redb`'s `no_std` support, and any secondary-index or
query-planning layer over `redb`'s tables.
