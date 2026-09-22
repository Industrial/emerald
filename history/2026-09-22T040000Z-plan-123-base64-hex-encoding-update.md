2026-09-22T04:00:00Z

# Plan 123 — Base64 & Hex Encoding — Update (2026-09-22, same-day session)

This is an update record for `history/2026-09-21T203200Z-plan-123-base64-hex-encoding.md`
(the original plan text, authored 2026-09-21) — kept as a separate,
dated file per this batch's own append-only convention rather than
editing the original, so the original's own Decision log stands
unmodified as written. Read that file first for the full original
design; this file records what was actually implemented and exactly
how it diverged.

## Status: implemented, all five leaves done, one real scope correction

Real crate check re-verified at THIS session's own authoring time
(2026-09-22, via lib.rs): `base64` 0.23.1 (Aug 4, 2026), #4 in
Encoding, 123,729,640 downloads/month, used in 95,248 crates,
MIT/Apache; `hex` 0.4.3 (Mar 3, 2021, stable/unchanged since), #52 in
Encoding, 53,852,713 downloads/month, used in 44,269 crates,
MIT/Apache — both consistent with the original plan text's own
same-week numbers. No RustSec advisory found against either crate
(checked directly, not assumed). `crates/emerald-rt/DEPENDENCIES.md`
gained two more real rows.

## The one real, disclosed scope correction: no `Bytes` type exists

The original plan's own text specifies `Bytes` (a `(ptr, len)`
Emerald-facing binary-buffer type, per plan 92's ABI convention) as
the type on both sides of every function here — `Base64.encode(data:
Bytes): String`, `Base64.decode(s: String): Result[Bytes, String]`,
and so on. Checked directly against `crates/emerald-sema/src/lib.rs`'s
own `Type` enum before writing any code (not assumed from the
original plan's own confident phrasing): **no `Bytes` type exists
anywhere in this compiler.** Plan 92's own history doc independently
confirms this is a real, known, previously-named gap, not an oversight
this session discovered fresh — its own "Not yet decided (blocking
EXECUTE)" section states plainly: "Whether a genuine Emerald-facing
`Bytes`/binary-literal type... belongs to this batch at all, or is
deferred indefinitely until a specific domain plan... actually needs
to construct, not just consume, non-UTF-8 Emerald-source data. This
plan deliberately does not decide that question."

Building a full `Bytes` type (a new `Type`/`ValKind` variant, a
length-prefixed packed-byte representation distinct from both
`Array[T]`'s 8-byte-per-element layout and `String`'s bare
null-terminated pointer, `String`<->`Bytes` conversions, FFI
marshaling changes across every existing `(ptr, len)`-consuming native
function) is a real, separate language-surface feature — comparable in
scope to `derive Serializable`/`Iterable[T]` earlier in this same
session's own work, not a small addition folded silently into "the
most mechanical plan in this batch." Rather than take that on as an
undisclosed side effect, every function in this plan operates on
Emerald's existing `String` instead:

- `Base64.encode(s: String): String` / `.encode_no_pad` / `.encode_
  url_safe` / `.encode_url_safe_padded` — treats `s`'s own bytes (via
  the same `(ptr, len)`-via-`CStr` pattern every other `emerald-rt`
  `String`-consuming intrinsic already uses) as the payload to encode.
  Never fails.
- `Base64.decode(s: String): Result[String, String]` and its three
  siblings — decodes `s`, then re-validates the decoded bytes as UTF-8
  before handing them back as a `String`. `Err` covers both a genuine
  base64 syntax error (wrong padding, out-of-alphabet character) AND
  decoded-bytes-aren't-valid-UTF-8 (a real, disclosed narrowing: this
  only correctly round-trips input whose ORIGINAL bytes were
  themselves valid, NUL-free UTF-8 text — exactly the shape the
  original plan's own Concrete Proof already used, `"hello world"`).
- `Hex.encode`/`.encode_upper`/`.decode` — identical shape.

This is real, useful capability for the base64url/JWT-identifier and
hex/digest-display use cases the original plan's own Decision log
already named as motivating (both are printable-text shapes, not
arbitrary embedded-NUL binary blobs) — narrower than originally
scoped, not a token gesture. Arbitrary binary round-tripping is
deferred until a real `Bytes` type lands, exactly per plan 92's own
still-open question.

## A second, small, genuinely necessary addition: nothing new needed

Unlike `derive Serializable` (which needed a brand-new `Int64#to_f`/
`Float64#to_i` conversion pair as a real prerequisite), this plan
needed no new language-level intrinsic beyond the `Base64`/`Hex`
reserved-namespace dispatch itself — every function's own signature is
`String -> String` or `String -> Result[String, String]`, both shapes
this compiler already fully supports (`Json.parse`'s own `Result[T,
String]` construction path, reused directly).

## Implementation

`crates/emerald-rt/src/encoding.rs` (new module): 11 functions —
`base64_encode`/`_no_pad`/`_url_safe`/`_url_safe_padded` (encode side,
4), matching `_decode` siblings (4), `hex_encode`/`_encode_upper`/
`_decode` (3). Every decode function shares one `decode_with` helper
parameterized over `&impl base64::engine::Engine`, avoiding four
near-identical copies. `alloc_and_copy_str`/`emerald_rt_result_ok`/
`emerald_rt_result_err_str` (plan 91/92's own established helpers) are
reused directly — zero new Rust-side infrastructure. 5 `#[test]`s
against RFC 4648's own worked example plus a malformed-input case for
each of base64/hex.

`crates/emerald-rt/src/lib.rs`: 11 new `#[no_mangle] pub unsafe extern
"C" fn emerald_rt_*` wrappers, each `catch_and_raise`-wrapped per plan
91's mandate, dispatched by exact free-function name exactly like
`Json`/`Log`/`File`.

`crates/emerald-sema/src/lib.rs`: two new `Expr::MethodCall` arms
(`Base64`, `Hex`), the identical reserved-namespace static-call shape
`Json`/`Log`/`File` already use, checked immediately after `Json`'s own
arm.

`crates/emerald-codegen/src/lib.rs`: matching `build_method_call`
dispatch (11 new `Ctx` fields, 11 new `module.add_function`
declarations, two new `if recv_name == "Base64"/"Hex"` blocks) — no
codegen-level surprises; this is the same File/Json/Log pattern
copied, not a new mechanism.

## Concrete proof, verified end to end

`examples/base64_hex_encoding.em` — RFC 4648's own worked example
(`"hello world"`), matching the original plan's own predicted output
exactly: `aGVsbG8gd29ybGQ=` (standard, padded), `hello world`
(round-tripped), `aGVsbG8gd29ybGQ` (url-safe, unpadded), `68656c6c6f
20776f726c64` (lowercase hex), `hello world` (hex round-trip), plus a
real negative proof (malformed base64 reaching `Err` with a real,
non-empty library error message). Verified via `cargo nextest`, not
assumed from the buffer-layout reasoning alone.

The same real, disclosed grammar correction every other example this
session found applies here too: the original plan's own Concrete Proof
used `case`/`when`/`else`, which do not exist in this grammar (removed
per plan 71) — the real syntax is `match X do Ok(v) do ... end Err(e)
do ... end end`.

Full workspace gate: `cargo nextest run --workspace` (966/966, 2
skipped — 6 new: 5 in `emerald-rt`, 1 in `emerald-cli`), `cargo clippy
--workspace --all-targets` (clean), `treefmt` (0 changed), `cargo
audit` (same 5 pre-existing, unrelated, already-triaged warnings — no
new finding from `base64`/`hex`).

## Explicitly out of scope, disclosed (unchanged from the original plan, plus one addition)

Everything the original plan's own "Out of scope" bullet already
named (base32/other `data-encoding` formats, streaming/incremental
encode-decode, custom alphabets, `no_std`/`alloc`-only configuration)
still applies unchanged. Added by this update: **arbitrary binary
`Bytes` round-tripping**, deferred until a real `Bytes` type exists —
see the Decision-log-equivalent section above for the full account of
why this wasn't attempted as a silent scope expansion.
