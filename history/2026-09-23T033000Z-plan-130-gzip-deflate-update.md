2026-09-23T03:30:00Z

# Plan 130 — Gzip/Deflate/Zlib Compression — Update

Update record for `history/2026-09-21T203900Z-plan-130-gzip-deflate.md`
(the original plan text) — kept as a separate, dated file per this
batch's own append-only convention rather than editing the original.
Read that file first for the full original design; this file records
what was actually implemented and exactly how it diverged.

## Status: implemented, all five leaves done

```
leaf-emerald-rt-flate2-dependency:              done
leaf-one-shot-compress-decompress:               done
leaf-emerald-surface-gzip-deflate-zlib-modules:  done
leaf-streaming-reader-writer-handles:            done
leaf-rust-tests-and-proof-example:               done
```

## The two "Not yet decided (blocking EXECUTE)" items — resolved

1. **`Bytes`'s real Rust-level representation** — plan 109 (Cryptographic
   Hashing), authored earlier the same session (`2026-09-21T201800Z`,
   before this plan's own `2026-09-21T203900Z`), had already landed a
   real `Bytes` type by the time this plan reached EXECUTE: a
   compiler-synthesized `Int64` newtype (`Type::Newtype("Bytes",
   Int64)`), backed at codegen by a bare heap pointer to a `[len: i64]
   [data]` block — never a `crate::handle` registry id, since a `Bytes`
   value holds no Rust-side resource. This plan's own `emerald_rt_
   gzip_compress`/`.decompress` (and siblings) signatures are written
   directly against that real, already-shipped shape (`bytes.rs`'s own
   `bytes_as_slice`/`bytes_from_slice`), confirming this plan's own
   hard structural dependency was already satisfied, not a guess.
2. **The `GzipWriter`/`GzipReader` handle table** — plan 93's
   `crate::handle` registry (`handle_alloc`/`handle_get_mut`/
   `handle_close`), the same global-with-a-mutex registry every other
   resource-handle type in this crate (`Regex`, `TlsStream`,
   `XmlReader`, ...) already uses. No bespoke handle table was
   designed for this plan.

## A real, disclosed dependency-graph finding

`flate2` 1.1.10 was already resolved into this workspace's own
`Cargo.lock` transitively — plan 100's `ureq` pulls it in via its own
default `gzip` feature. Verified this session via `cargo tree -p
emerald-rt -i flate2 -e features` (run BEFORE this plan's own `Cargo.
toml` edit): the already-active feature set resolves to exactly
`rust_backend` → `miniz_oxide` (`any_impl`, `runtime_detection`) — the
identical backend this plan's own Decision log independently chose and
pins by its concrete feature name, confirmed via `cargo tree`, not
merely asserted. This plan's `default-features = false, features =
["miniz_oxide"]` declaration on `emerald-rt`'s own `Cargo.toml` entry
is therefore a direct-dependency promotion with an explicit pin, not
new supply-chain surface — `Cargo.lock` itself only gained one line
(`"flate2",` added to `emerald-rt`'s own dependency list; no new
package block, no version/checksum change).

## Implementation

`crates/emerald-rt/src/gzip.rs` (new module, ~600 lines): one `Format`
enum (`Gzip`/`Deflate`/`Zlib`) and one generic `WriterInner`/
`ReaderInner` pair shared by all three formats (mirroring `encoding.
rs`'s own "one `decode_with` helper parameterized over the engine"
convention, rather than three near-identical copies) — `Gzip`/
`Deflate`/`Zlib`'s six one-shot `.compress`/`.decompress` functions and
`GzipWriter`/`GzipReader`'s (+ siblings) eighteen streaming functions
are all thin wrappers around this shared core. `WriterState` wraps its
`WriterInner` in an `Option` specifically so `.close()` can `.take()`
real ownership out from behind `crate::handle::handle_get_mut`'s `&mut`
callback shape, to call flate2's own consuming `{Gz,Deflate,Zlib}
Encoder::finish(self)` (which flushes the format's own trailer bytes —
gzip's CRC32+size, zlib's Adler-32) — a real, necessary wrinkle
`TlsStream`'s own `#close` never had to solve, since `rustls::
StreamOwned` needs no analogous consuming finish step. 9 `#[test]`s:
round-trip + real-compression assertions for all three formats, a
zero-length input, a non-UTF8 byte sequence, a byte-for-byte pin
against calling `flate2` directly, raw-deflate's own header-less shape
distinguished from gzip's/zlib's, a real `Err` (not a panic) feeding
gzip bytes to `Deflate.decompress`, and a real file-backed writer/
reader round trip.

`crates/emerald-rt/src/lib.rs`: `mod gzip;` plus 24 new `#[no_mangle]
pub unsafe extern "C" fn emerald_rt_*` wrappers, each `catch_and_
raise`-wrapped per plan 91's mandate.

`crates/emerald-sema/src/lib.rs`: `Gzip`/`Deflate`/`Zlib` as a
reserved-namespace static-call arm (`.compress`/`.decompress`, both
`Bytes -> Bytes`, the identical shape `Sha256`/`AesGcm256` already
establish) — `.decompress` returns a bare `Bytes`, NOT `Result[Bytes,
String]`: a corrupt/mismatched-format stream raises plan 92's
`NativeError` channel directly, the same "one underlying I/O call,
raise rather than `Result`" posture `TcpStream#read`/`TlsStream#read`
already establish, matching this plan's own Concrete Proof text
verbatim (`restored: Bytes = Gzip.decompress(compressed)`, never
through a `match`/`Result`). `GzipWriter`/`DeflateWriter`/`ZlibWriter`/
`GzipReader`/`DeflateReader`/`ZlibReader` registered as six `Int64`-
newtype classes (the identical `TlsStream`/`TlsListener` shape),
`.open(path: String)` as a second reserved-namespace static arm,
`.write_chunk`/`.close` and `.read_chunk`/`.close` carved out of the
ordinary newtype `.value`-only restriction.

`crates/emerald-codegen/src/lib.rs`: matching `NEWTYPE_UNDERLYING`
entries, 24 new `Ctx` fields, 24 new `module.add_function` declarations,
two new static-call dispatch blocks (`Gzip`/`Deflate`/`Zlib`;
`*Writer`/`*Reader.open`), two new instance-method dispatch blocks
(`*Writer#write_chunk`/`#close`; `*Reader#read_chunk`/`#close`) — the
same `TlsStream`/`TlsListener`/`TcpListener.bind` pattern copied, not a
new mechanism. One real, disclosed correction found only by compiling:
this dispatch function's own `recv_name`/`method` parameters are
`&String`, not `&str` (unlike the sibling instance-method-dispatch
function, which does take `&str`) — every `matches!`/`match` against a
string literal needed an explicit `.as_str()`, caught immediately by
`cargo check`, not a design change.

## A real, disclosed adaptation to the Concrete Proof

The plan's own literal Concrete Proof text assumes a `Bytes` surface
richer than what actually exists today: `.length` and a content-aware
`==`. Checked directly against `emerald-sema`'s own `Bytes` carve-out
before writing the example (not assumed from the plan's own confident
phrasing): `Bytes` exposes exactly one instance method,
`.to_hex(): String` (plan 109's own scope). A bare `==` between two
`Bytes` newtype handles would compare their own underlying heap
POINTERS, not content — silently printing `false` for a genuinely
correct round trip through two different heap allocations. `examples/
gzip_roundtrip.em` therefore compares each `Bytes` value's own `.to_hex
()` `String` instead (real content comparison/real length, and two
equal-content `Bytes` values always hex-encode to the same `String` —
a faithful stand-in, not a narrowed proof). Two further real, disclosed
grammar corrections, both already-established findings from earlier
plans this same session (plan 109's `crypto_hashing_proof.em`, plan
193's `set_deque_priority_queue.em`), reused verbatim rather than
rediscovered: a `.method(...)` call's receiver can never be a string
literal or the bare result of a previous, non-block-attached
`.method(...)` call (every intermediate value is bound to a local
first), and `puts` accepts only `Int64`/`Float64`/`String` (every
`Boolean` comparison below needs `"#{...}"` string interpolation).
None of this changes the example's own expected output: `true`,
`true`, `true`, `true`.

## Gate

`cargo build --workspace` (clean), `cargo clippy --workspace
--all-targets` (clean), `treefmt` (0 files changed), `cargo nextest run
--workspace` — 1174/1175 passed, 2 skipped, 1 pre-existing failure
(`emerald-driver::cache::tests::corrupting_the_cached_object_file_
forces_a_real_recompile_not_an_error`) confirmed unrelated to this
plan by reproducing it in isolation against plain, unmodified HEAD
(`cargo test -p emerald-driver corrupting_the_cached_object_file_
forces_a_real_recompile_not_an_error`, same failure, same assertion,
same crate this plan never touches) — a pre-existing, environment-
specific flake, not introduced here. `cargo nextest run --workspace`
at full default parallelism also hit a transient `No space left on
device` failure extracting the embedded `emerald-rt` archive to `/tmp`
(this sandbox's own known `/tmp`-capacity issue, several concurrent
agents' own build artifacts observed filling the same shared `/tmp`)
— resolved by pointing `TMPDIR` at a roomier directory for the test
run, not a real code issue.

## Explicitly out of scope (unchanged from the original plan)

Everything the original plan's own "Out of scope" bullet already named:
no compression-level parameter, no gzip multi-member/concatenated-
stream support, no `.gz` filename/mtime header metadata surface, no
dictionary-based deflate. `.tar.gz` composition remains plan 132's job.
