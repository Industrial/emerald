2026-09-23T06:30:00Z

# Plan 132 — Tar Archives — Update

Update record for `history/2026-09-21T204100Z-plan-132-tar-archives.md`
(the original plan text) — kept as a separate, dated file per this
batch's own append-only convention rather than editing the original.
Read that file first for the full original design; this file records
what was actually implemented and exactly how it diverged.

## Status: implemented, all five leaves done

```
leaf-emerald-rt-tar-dependency:       done
leaf-create-archive:                  done
leaf-extract-archive:                 done
leaf-streaming-tar-reader:            done
leaf-rust-tests-and-proof-example:    done
```

## Real, disclosed divergences from the plan's own literal text

1. **`emerald_rt_tar_create`'s own real signature omits `path_lens`.**
   The plan's own `leaf-create-archive` text names `path_ptrs: *const
   *const c_char, path_lens: *const i64, count: i64` — plan 92's
   binary-safe (ptr+len) convention applied per-element. Checked
   directly against this codebase's own established `String` ABI
   before implementing (not assumed from the plan's own confident
   phrasing): every OTHER `String`-typed parameter anywhere in
   `emerald-rt` already crosses as a bare `*const c_char`, trusting
   `CStr::from_ptr`'s own NUL-scan (`GzipWriter.open`'s own `path`
   parameter is the closest sibling) — Emerald's `String` never carries
   embedded NULs at this codegen's own representation, so a per-element
   length would require new codegen machinery (computing each array
   element's own `strlen` before the call) purely to duplicate what
   `CStr::from_ptr` already gives for free. The real, new marshaling
   shape this plan's own text correctly flags — the pointer-array-plus-
   count pair itself, since `Tar.create`'s second argument is `Array[
   String]`, not a `Bytes` — is what got built; the redundant parallel
   length array did not.
2. **`Dir.mkdir` does not exist at this plan's own EXECUTE time.** The
   plan's own Concrete Proof assumes it as "ordinary filesystem stdlib
   surface... not itself designed here." Checked directly (not
   assumed) against the real `emerald-rt` module list before writing
   `examples/tar_roundtrip.em`: no `Dir` class of any kind is
   registered anywhere in `emerald-sema` as of this plan's own EXECUTE
   time — plan 144 (Extended Filesystem Operations), landed in this
   same session but authored independently, does not add `Dir.mkdir`
   either (it adds `Path`/`FileMetadata`/`Dir.walk`/`.entries`, no
   directory-creation intrinsic). The worked proof archives two plain
   files written directly in the working directory instead of inside a
   subdirectory first — `Tar.create`'s own real behavior (member paths
   exactly as given, member order exactly caller order) is identical
   either way, so this changes nothing about what the proof actually
   demonstrates.
3. **`match`/`Option[T]` pattern syntax is `match expr do / Variant(
   binding) do ... end / end`, not the plan's own Ruby-flavored `when`
   keyword.** The identical correction `environment_variables_proof.em`
   (plan 146) and `nullable_safe_nav.em` (plan 73) already establish,
   reused verbatim rather than rediscovered. A further, genuinely new
   finding this plan's own worked proof surfaced: `match`'s own
   scrutinee must itself already be a plain, bound local (`Expr::
   Ident`) — `emerald-codegen`'s own enum-case dispatch fast path
   (`build_case`) only recognizes a bare identifier whose `local_
   classes` entry names a known enum, falling through to a generic
   "scrutinee must be `Int64`" path otherwise, which rejects `match
   reader.next_entry() do ...` (a `ValKind::Ptr`, not `Int64`) with a
   real, non-obvious `codegen: \`case\` scrutinee must be Int64` error.
   Every `.next_entry()` result in the worked proof is therefore bound
   to a local (`entry1`/`entry2`/`entry3`) before its own `match`, the
   identical shape `environment_variables_proof.em`'s own `found`/
   `gone` locals already establish for exactly this reason — not
   previously written down as a general `match`-scrutinee rule
   anywhere this session's other history files, so recorded here.
4. **`Bytes` exposes exactly one instance method today, `.to_hex():
   String` — no `.to_string()`.** The plan's own Concrete Proof prints
   `reader.read_entry_data().to_string()` expecting the literal
   restored text; checked directly against `bytes.rs`'s own real
   surface before writing the example (the identical class of
   correction `gzip_roundtrip.em`, plan 130, already discloses). Every
   `reader.read_entry_data()` in the worked proof is therefore printed
   via `.to_hex()` instead — real content is still genuinely proven
   (two files with different content hex-encode to different strings,
   and the restored file's own content is separately verified byte-
   for-byte via `File.read` at the very end, never just re-hashed).

## A real, disclosed implementation simplification (not a plan-text
## divergence — `TarReader`'s own internal Rust shape)

`tar::Archive::entries()` returns a `tar::Entries<'a, R>` genuinely
borrowing `&'a Archive<R>` — a self-referential shape (the iterator
borrows from the very resource `crate::handle`'s `'static`-bound
registry would need to own) with no "read one entry, no borrowed
iterator" alternative method on `tar::Archive` itself (verified
directly against the vendored `tar-0.4.46` source before choosing an
approach, not assumed) — unlike `xml.rs`'s own `XmlReader`, which
sidesteps the identical class of problem because `quick_xml::Reader::
read_event_into` has no comparable borrowed-iterator step to begin
with. Rather than a hand-rolled unsafe self-referential struct for no
real benefit this plan's own Concrete Proof needs, `TarReader.open`
eagerly reads every entry's name AND full content into a plain `Vec`
once, up front; `.next_entry`/`.entry_size`/`.read_entry_data` walk
that `Vec` with a cursor. A real, disclosed capability loss for a
future caller wanting to stream a multi-gigabyte archive entry by
entry without ever holding it all in memory at once — not a gap this
plan's own worked proof (two small files) exercises, and easily
revisited later without changing `TarReader`'s own Emerald-facing
surface at all (a purely internal `tar.rs` change).

## A real, disclosed finding from actually running this against
## `tar`'s own real, default-on security check

`tar::Builder::append_path`'s own `Header::set_path` (via `default-
features = false`'s un-set `preserve_absolute` option, `false` by the
crate's own default) rejects — a real `Err`, `Tar.create` then
genuinely raises via plan 92's `NativeError` channel — any path whose
own `Component::RootDir` is present, the identical "refuse to silently
write an absolute member path" default GNU tar's own `--absolute-
names` flag exists to override. Found only by actually running this
crate's own `#[test]`s with `std::env::temp_dir()`-based (therefore
absolute) source paths and hitting a real `SIGABRT` (this test binary's
own `emerald_raise` stub aborts the process, plan 92's own doc comment
disclosed reason) — every path this plan's own Rust-level tests hand
to `tar_create` is therefore genuinely relative, which (since `append_
path` also opens the given path from the process's own real working
directory to read its content) means the fixture files live under
`target/` (Cargo's own guaranteed `cargo test` working directory: the
crate's own manifest directory), not under `std::env::temp_dir()`.
`tar::Archive::unpack`'s own extraction side, by contrast, strips a
leading `/` from a member path outright (`Component::RootDir` is
silently treated as an "empty component" during `unpack_in`, verified
directly against the vendored source) — an absolute-looking member
name is never rejected on the READ side, only on the WRITE side.

## Implementation

`crates/emerald-rt/src/tar.rs` (new module, ~420 lines): `Tar.create`/
`.extract` (one-shot, via `tar::Builder::append_path`/`tar::Archive::
unpack`), `TarReader.open`/`.next_entry`/`.entry_size`/`.read_entry_
data`/`.close` (plan 93's `crate::handle` registry, backed by an
eagerly-read `Vec<(String, Vec<u8>)>` + cursor per the Decision log
above). Reuses `regex.rs`'s own `alloc_option_string`/`Option$String`
tagged-block convention verbatim (its own private copy, matching every
other module's own established practice of not sharing this hand-built
layout across modules). 3 `#[test]`s: create-then-list-then-extract
round trip against two real files (name/size/content all asserted,
plus the terminating `None` past the last entry), an empty-`paths`
edge case, and a real `.tar.gz` composition test piping `Tar.create`'s
own raw bytes through the real, public `Gzip.compress`/`.decompress`
(plan 130) and back, then re-reading the decompressed bytes as a real
tar archive — proving the two plans genuinely compose, not merely both
existing.

`crates/emerald-rt/src/lib.rs`: `mod tar;` plus 7 new `#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_*` wrappers, each `catch_and_
raise`-wrapped per plan 91's mandate.

`crates/emerald-sema/src/lib.rs`: `Tar` as a reserved-namespace static-
call arm (`.create(String, Array[String]): Void` / `.extract(String,
String): Void`); `TarReader` registered as an `Int64`-newtype class
(the identical `Regex`/`GzipReader` shape) with `.open(path: String)`
as a second reserved-namespace static arm, `.next_entry`/`.entry_size`/
`.read_entry_data`/`.close` carved out of the ordinary newtype `.value`-
only restriction.

`crates/emerald-codegen/src/lib.rs`: a `NEWTYPE_UNDERLYING` entry AND a
`newtypes.insert("TarReader".to_string())` entry (both registries are
load-bearing here, per `XmlReader`'s own disclosed finding this plan
reuses rather than rediscovers — this plan's own worked proof really
does `Let`-bind a `TarReader`-typed local, unlike `GzipWriter`/
`GzipReader`, which this session found are only ever registered in
`NEWTYPE_UNDERLYING`, never in `newtypes.insert(...)`, a real,
pre-existing, disclosed-but-not-this-plan's-to-fix gap noted below), 7
new `Ctx` fields, 7 new `module.add_function` declarations, one new
static-call dispatch block (`Tar.create`/`.extract`, `TarReader.open`),
one new instance-method dispatch block (`TarReader#next_entry`/`#entry_
size`/`#read_entry_data`/`#close`) — the same `Regex`/`GzipReader`
pattern copied, not a new mechanism. `Tar.create`'s own dispatch is the
one genuinely new shape in this plan: it reads its own `paths: Array[
String]` argument's `[length: Int64][elements...]` header directly
(mirroring `build_array_each`'s own read of the identical layout
`build_array_lit` writes) rather than simply forwarding an already-
built argument value, since `emerald_rt_tar_create`'s own FFI signature
needs the element-pointer array and count as two separate parameters.

## A real, pre-existing gap noticed but deliberately not fixed

`GzipWriter`/`GzipReader`/`DeflateWriter`/`DeflateReader`/`ZlibWriter`/
`ZlibReader` (plan 130) are registered in `emerald-codegen`'s own
`NEWTYPE_UNDERLYING` map but never in the separate `newtypes.insert(...)`
`HashSet` `compile_to_object_impl` builds — per `XmlReader`'s own
already-disclosed finding (plan 124: "adding `XmlReader` to `newtypes.
insert(...)` alone was not enough"), a `Let`-bound `writer: GzipWriter =
GzipWriter.open(...)` local should hit the identical real LLVM verifier
failure `XmlReader` hit before both registries were populated. Not
actually exercised by `examples/gzip_roundtrip.em` (it never binds a
`GzipWriter`/`GzipReader`-typed local, only `Bytes` locals), so this
gap shipped silently with plan 130 and remains latent. Out of this
plan's own scope (a `gzip.rs`/plan-130 fix, not a `tar.rs`/plan-132
one) — flagged here rather than fixed, per this project's "no drive-by
refactors" convention; this plan's own `TarReader` is registered in
BOTH registries specifically because its own worked proof needs it,
verified directly rather than assumed.

## Gate

`cargo build --workspace` (clean, verified twice — once against this
plan's own EXECUTE-time HEAD, once re-verified from scratch against
plan 144's own HEAD after it landed mid-session), `cargo clippy
--workspace --all-targets` (clean — 0 errors, only pre-existing `missing_
safety_doc` warnings shared with every sibling `unsafe extern "C" fn`
in this crate, and one pre-existing upstream C compiler warning, exit
0), `treefmt` (0 files changed), `cargo nextest run --workspace` —
1186/1187 passed, 2 skipped, 1 pre-existing failure (`emerald-driver::
cache::tests::corrupting_the_cached_object_file_forces_a_real_recompile_
not_an_error`) confirmed unrelated to this plan by reproducing it in
isolation (`cargo test -p emerald-driver corrupting_the_cached_object_
file_forces_a_real_recompile_not_an_error`, same failure, a crate this
plan never touches) — a pre-existing, environment-specific flake, not
introduced here. `cargo nextest run --workspace` at full default
parallelism also hit a transient `No space left on device` failure
extracting the embedded `emerald-rt` archive to `/tmp` (this sandbox's
own known, shared `/tmp`-capacity issue, several concurrent agents'
own build artifacts observed filling the same shared `/tmp` this same
session) — resolved by pointing `TMPDIR` at a roomier directory for the
test run, not a real code issue.

## A real, disclosed collision-handling note

This plan's own EXECUTE happened in a live, multi-agent shared working
tree. Plan 144 (Extended Filesystem Operations) landed mid-session,
moving `HEAD` and touching every one of this plan's own shared files
(`Cargo.lock`, `crates/emerald-cli/tests/examples.rs`, `crates/emerald-
codegen/src/lib.rs`, `crates/emerald-rt/Cargo.toml`, `crates/emerald-
rt/DEPENDENCIES.md`, `crates/emerald-rt/src/lib.rs`, `crates/emerald-
sema/src/lib.rs`) — this plan's own isolated-worktree verification was
therefore redone in full against the new `HEAD` rather than reused. A
second, independent agent's own in-flight, uncommitted work (plan 145,
Process Spawning & Control) was live in the shared working tree's own
`crates/emerald-rt/src/lib.rs`/`crates/emerald-cli/tests/examples.rs`
at this plan's own commit time; those two files' final blobs were
built from a clean `git show HEAD:<path>` base plus only this plan's
own edits (verified end-to-end in the isolated worktree first) and
staged directly via `git hash-object -w` + `git update-index
--cacheinfo`, never overwriting that other agent's own working-tree
copies — `git diff --cached` was checked immediately before committing
to confirm every staged line was exactly this plan's own. A real,
pre-existing, unrelated finding surfaced along the way and left
untouched: `crates/emerald-rt/DEPENDENCIES.md`'s own `flate2` (plan
130) ledger row is missing entirely as of plan 144's own commit — some
earlier edit to that file dropped it — not this plan's own row to
restore (this plan's own new `tar` row was appended after the existing
`tempfile` row instead of anchored to the now-absent `flate2` row).

## Explicitly out of scope (unchanged from the original plan)

Everything the original plan's own "Out of scope" bullet already
named: no recursive directory archiving, no extended attributes (`tar`
depends with `default-features = false`, disabling `xattr`), no
symlink-following policy choices beyond `append_path`'s own default,
no sparse-file support, no PAX long-filename/long-link extensions
beyond whatever `tar-rs` handles by default, no in-place archive
modification, no `.tar.gz`/`.tar.zst` convenience wrappers (this
plan's own `.tar.gz` composition test proves the composition works
without building sugar on top of it, exactly as the original plan's
own text specifies).
