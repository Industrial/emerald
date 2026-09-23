2026-09-23T07:35:00Z

# Plan 133 — Zip Archives — Update

Update record for `history/2026-09-21T204200Z-plan-133-zip-archives.md`
(the original plan text) — kept as a separate, dated file per this
batch's own append-only convention rather than editing the original.
Read that file first for the full original design; this file records
what was actually implemented and exactly how it diverged, plus a
genuinely eventful collision-handling story this plan's own EXECUTE hit.

## Status: implemented, all five leaves done

```
leaf-emerald-rt-zip-dependency:       done
leaf-create-archive:                  done
leaf-extract-archive:                 done
leaf-streaming-zip-reader:            done
leaf-rust-tests-and-proof-example:    done
```

## Real, disclosed divergences from the plan's own literal text

1. **`Dir.mkdir` still does not exist at this plan's own EXECUTE time.**
   The plan's own Concrete Proof calls `Dir.mkdir("zip_demo_src")`
   first — checked directly (not assumed) against the real `dir.rs`
   before writing `examples/zip_roundtrip.em`: plan 144 (Extended
   Filesystem Operations), landed earlier this same session, adds only
   `Dir.walk`/`.walk_count`/`.entries`/`.entries_count`, never a
   `.mkdir`. The identical adaptation plan 132's own `tar_roundtrip.em`
   already makes for the identical reason: this example archives two
   plain files written directly in the working directory instead of
   inside a subdirectory first — `Zip.create`'s own real behavior
   (member paths exactly as given) is identical either way.
2. **`Bytes` still exposes exactly one instance method today, `.to_hex(
   ): String` — no `.to_string()`.** The plan's own Concrete Proof
   prints `reader.read_entry_data(i).to_string()`; every `reader.read_
   entry_data(i)` in the worked proof is printed via `.to_hex()`
   instead, the identical substitution `gzip_roundtrip.em`/`tar_
   roundtrip.em` already establish — real content is still genuinely
   proven (two files with different content hex-encode to different
   strings, and the restored file's own content is separately verified
   byte-for-byte via `File.read` at the very end).
3. **`puts` still accepts only `Int64`/`Float64`/`String`.** The final
   `Boolean` equality check needs `"#{...}"` string interpolation to
   print, not a bare `puts`, the same correction every prior `.em`
   proof in this batch already makes.
4. **No `match`/`Option[T]` adaptation was needed at all — a genuine,
   format-driven difference from plan 132, not an oversight.** Unlike
   `TarReader#next_entry` (`Option[String]`, `None` past the last
   entry), `ZipReader` is index-based per this plan's own Decision
   log — `entry_count()`/`entry_name(i)`/`entry_size(i)`/`read_entry_
   data(i)` all take/return plain, non-`Option` values, so the worked
   proof's `while i < n do ... end` loop needed none of `tar_roundtrip.
   em`'s own `match expr do / Variant(binding) do ... / end`
   restructuring. `var i: Int64 = 0` / `i += 1` (a real, mutable
   binding, distinct from `collections.em`'s own non-`var` re-`Let`
   loop-counter style) is the only loop-mechanics choice this example
   had to make, confirmed against `bitwise_and_assignment.em`'s own
   established `var`/`+=` precedent before writing it.

No other divergence: the crate ended up exactly as named in the
Decision log (`zip` 8.6.0, `default-features = false`, `deflate-
flate2-zlib-rs`), and the real `zip-rs/zip2` 8.6.0 API (verified
directly against the vendored source before writing `zip.rs`, not
assumed from memory) matched the plan's own cited shape closely
enough that no API-name corrections were needed: `zip::write::
SimpleFileOptions::default().compression_method(CompressionMethod::
Deflated)`, `ZipArchive::new`/`.len`/`.by_index`/`.extract`, `ZipFile::
name`/`.size`/`.compression`/`Read::read_to_end`.

## A real, disclosed implementation choice — `ZipReader` stores the
## live `zip::ZipArchive<File>` itself, no `TarReader`-style eager read

Per this plan's own Decision log: a `.zip` file's central directory is
parsed once by `ZipArchive::new` and gives `.len()`/`.by_index(i)` as
real, cheap, already-available operations with no comparable self-
referential-iterator problem `TarReader`'s own `tar::Entries<'a, R>`
has. `ZipReader.open` therefore stores the live, boxed `zip::
ZipArchive<File>` directly in plan 93's `crate::handle` registry;
`.entry_count`/`.entry_name`/`.entry_size`/`.read_entry_data` each call
`.by_index(i)` fresh through `handle_get_mut`. No eager `Vec<(String,
Vec<u8>)>` pre-read, no cursor field — genuinely simpler than
`TarReader`'s own Rust-side shape, a direct, honest reflection of the
real structural difference between the two formats this plan's own
Decision log already predicted, not a new finding made mid-
implementation.

## Implementation

`crates/emerald-rt/src/zip.rs` (new module, 355 lines): `Zip.create`/
`.extract` (one-shot, via `zip::ZipWriter`/`zip::ZipArchive::extract`),
`ZipReader.open`/`.entry_count`/`.entry_name`/`.entry_size`/`.read_
entry_data`/`.close` (plan 93's `crate::handle` registry, backed by
the live `ZipArchive<File>` itself — see above). 3 `#[test]`s: create-
then-list-then-extract round trip against two real files (name/size/
content all asserted), an empty-`paths` edge case, and a real
assertion that a compressible entry's own stored `CompressionMethod`
is genuinely `Deflated` (both via the archive's own smaller-than-input
byte count AND `ZipFile::compression()` directly) — proving the
`deflate-flate2-zlib-rs` feature is actually wired up, not silently
falling back to `Stored`.

`crates/emerald-rt/src/lib.rs`: `mod zip;` plus 8 new `#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_*` wrappers, each `catch_and_
raise`-wrapped per plan 91's mandate.

`crates/emerald-sema/src/lib.rs`: `Zip` as a reserved-namespace static-
call arm (`.create(String, Array[String]): Void` / `.extract(String,
String): Void`); `ZipReader` registered as an `Int64`-newtype class
with `.open(path: String)` as a second reserved-namespace static arm,
`.entry_count`/`.entry_name(Int64)`/`.entry_size(Int64)`/`.read_entry_
data(Int64)`/`.close` carved out of the ordinary newtype `.value`-only
restriction — index-based signatures, the real API-shape difference
from `TarReader` this plan's own Decision log names.

`crates/emerald-codegen/src/lib.rs`: `ZipReader` registered in BOTH
newtype registries (`NEWTYPE_UNDERLYING` AND the separate `newtypes.
insert("ZipReader".to_string())` near `compile_to_object_impl`) —
verified directly, not assumed, against the exact gap `TarReader`'s
own implementing agent found and `GzipWriter`/`GzipReader`/`DeflateWriter`/
`DeflateReader`/`ZlibWriter`/`ZlibReader` had silently shipped without:
this plan's own worked proof genuinely `Let`-binds `reader: ZipReader =
ZipReader.open(...)`, so both registries are load-bearing here, caught
by the isolated-worktree `examples/zip_roundtrip.em` run passing
end-to-end (a real LLVM verifier failure would have surfaced
immediately at that Let-binding if either registry were missing). 8
new `Ctx` fields, 8 new `module.add_function` declarations, one new
static-call dispatch block (`Zip.create`/`.extract`, `ZipReader.open`
— `Zip.create`'s own `paths: Array[String]` argument unpacked into its
raw `elements_base`/`count` pair, the identical shape plan 132's `Tar.
create` already established), one new instance-method dispatch block
(`ZipReader#entry_count`/`#entry_name`/`#entry_size`/`#read_entry_
data`/`#close`, index-arg-aware via the same `call_args`-accumulation
loop `GzipReader#read_chunk` already establishes for a method taking
an extra scalar argument).

## A genuinely eventful collision-handling story — three follow-up
## commits, one real near-miss caught and fixed, not shipped broken

This plan's own EXECUTE happened in an extremely active live, multi-
agent shared working tree — `HEAD` moved twice mid-implementation
(plan 130's own newtype-registry fix, then plan 145, Process Spawning
& Control) before this plan's own first commit, and a THIRD agent
(plan 147, Temporary Files & Directories) was actively editing
`crates/emerald-codegen/src/lib.rs`/`crates/emerald-sema/src/lib.rs`/
`crates/emerald-rt/src/lib.rs`/`crates/emerald-rt/Cargo.toml`/
`crates/emerald-rt/DEPENDENCIES.md` — the identical `TarReader`-
adjacent insertion points this plan also uses — at effectively the
same moment this plan's own commits were being made.

The first commit (`c23efe6`) used this task's own documented
technique (`git hash-object -w` + `git update-index --cacheinfo` +
`git diff --cached --stat` verification + `git commit -- <paths>`) and
looked clean by that check, but two real, disclosed things went wrong
that the `--stat` check alone did not catch:

1. `git commit -m "..." -- <9 paths>` silently committed only 6 of the
   9 named paths (`crates/emerald-codegen/src/lib.rs`, `crates/
   emerald-sema/src/lib.rs`, and `crates/emerald-cli/tests/examples.rs`
   were dropped entirely) — a real race where plan 147's own
   concurrent `git add`/staging of those exact same files landed in
   the gap between this plan's own `git update-index` calls and the
   `git commit` invocation, even though both were issued back-to-back.
   A first follow-up commit attempt to restore them raced again (twice
   in a row) and, worse, ended up committing plan 147's own in-flight
   `Tempfile`/`Tempdir` codegen diff under this plan's own commit
   message instead of this plan's own `ZipReader` wiring.
2. `crates/emerald-rt/src/lib.rs` itself (66 insertions committed vs.
   88 expected) turned out to be entirely plan 147's own `mod
   tempfiles;` + six `emerald_rt_tempfile*`/`emerald_rt_tempdir*`
   wrapper functions, not this plan's own `mod zip;` + eight
   `emerald_rt_zip*` wrappers at all — not caught by the immediate
   post-commit `git status`/content checks (which checked the other
   three files, not this one), only found afterward by a fresh `cargo
   test` run against the actually-committed tree in a disposable
   worktree, hitting a genuine `couldn't read
   crates/emerald-rt/src/tempfile.rs: No such file or directory`
   compile error — `mod tempfiles;` with no corresponding file
   committed anywhere in this repo's history, since plan 147 had not
   committed anything of its own at any point.

**Both were real, disclosed mistakes this plan's own agent made and
then found and fixed before reporting done — not shipped broken.**
Every fix used the same escalation, in order of increasing
resistance to the observed race:
`git hash-object` + `git update-index --cacheinfo` + `git commit --
<paths>` (worked for the plan's very first commit's 6 files, raced on
the other 3); a tighter single-shell-invocation version of the same
sequence (raced again, identically); and finally `git commit-tree`
plumbing against a **private `GIT_INDEX_FILE`** (`git read-tree
<HEAD>` into a throwaway index file outside `.git/index`, `git
update-index --cacheinfo` into that private index only, `git
write-tree`, verified directly via `git show <tree>:<path> | grep`
against the resulting tree object *before* ever touching the shared
branch — a real, genuine content check, not a `--stat` line count —
then `git commit-tree` + `git update-ref refs/heads/main <new> <old>`,
an atomic compare-and-swap that fails safely, not silently, if `HEAD`
moved again first) — the only one of the three that never touches
the shared repository index at all, and therefore the only one immune
to a concurrent agent's own `git add`/staging activity on the same
paths. Two such commits (`1cd0071` for `codegen`/`sema`/`examples.rs`,
`aa21104` for `lib.rs`) closed both gaps.

At every step, plan 147's own actual working-tree files
(`crates/emerald-rt/src/tempfile.rs`, their own uncommitted edits to
`sema.rs`/`Cargo.toml`/`DEPENDENCIES.md`, `examples/temp_files_proof.
em`) were read from (to identify what needed to be reversed out of
this plan's own accidental commits) but never written to — every
correction operated on this repository's shared **git index/tree
objects** only, confirmed by `git status --porcelain` immediately
after each fix showing those files still present, still untracked/
modified exactly as that other agent had left them, ready for their
own eventual `git add`/commit. The final committed state (`aa21104`)
was re-verified end-to-end from scratch in a fresh, disposable
`git worktree`: `cargo build --workspace` clean, `cargo nextest run
-p emerald-rt -p emerald-codegen -p emerald-sema` (750/750 passed),
`cargo nextest run -p emerald-cli --test examples --test-threads 4`
(61/61 passed, including `zip_roundtrip`/`tar_roundtrip`/
`process_spawning_proof`/`gzip_roundtrip` all still green — full-
parallelism `--test-threads` default hit the sandbox's own pre-
existing, disclosed flakiness under heavy load, resolved by the same
reduced-parallelism re-run this repo's own tooling notes already
document), `cargo clippy --workspace --all-targets` (0 errors),
`treefmt --fail-on-change` (0 changed), `cargo audit --ignore
RUSTSEC-2023-0071` (0 new advisories against `zip`'s own dependency
graph).

## Gate

See the collision-handling section above for the full verification
narrative. Final state: `cargo build --workspace` clean, `cargo
nextest run -p emerald-rt -p emerald-codegen -p emerald-sema`
750/750 passed, `cargo nextest run -p emerald-cli --test examples
--test-threads 4` 61/61 passed, `cargo clippy --workspace
--all-targets` 0 errors (only pre-existing `missing_safety_doc`
warnings shared with every sibling `unsafe extern "C" fn` in this
crate), `treefmt --fail-on-change` 0 changed, `cargo audit --ignore
RUSTSEC-2023-0071` 0 new advisories.

## Explicitly out of scope (unchanged from the original plan)

Everything the original plan's own "Out of scope" bullet already
named: no `bzip2`/`lzma`/`xz`/`ppmd`/`zstd`-via-this-crate compression
methods, no AES encryption, no per-entry `Stored` vs `Deflated`
choice, no ZIP64-specific API surface beyond whatever `zip`'s own
DEFLATE path handles transparently, no streaming/incremental `Zip.
create`, no in-place archive modification.
