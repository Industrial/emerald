2026-09-21T21:02:00Z

---
name: Character Set / Encoding Conversion — `encoding_rs` for Legacy-Charset Interop
overview: "A new `Encoding` compiler-provided namespace — `Encoding.decode(raw, label): String`, `Encoding.decode_strict(raw, label): String?`, `Encoding.encode(text, label): String` — wrapping `encoding_rs` (the WHATWG Encoding Standard implementation that has shipped inside Firefox/Gecko since Firefox 56, verified this session at 0.8.41, 26,019 dependent crates) to move text between Emerald's native UTF-8 `String` and legacy single- and multi-byte encodings (Latin-1/windows-1252, Shift_JIS, GBK, EUC-JP, and every other WHATWG-registered label) for interop with external data sources — files, sockets, legacy databases — that were never UTF-8 to begin with. This is the first plan in the batch to produce a `String` value that deliberately does not satisfy `String`'s own documented UTF-8 contract, and it says so plainly rather than hiding the gap."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-encoding-rt-crate-and-lookup
    content: "Add `encoding_rs = \"0.8.41\"` to `crates/emerald-rt/Cargo.toml` ([dependencies], per plan 91's scaffold). Write `crates/emerald-rt/src/encoding.rs` with a private `fn resolve(label_ptr: *const c_char) -> Option<&'static encoding_rs::Encoding>` — `unsafe { CStr::from_ptr(label_ptr) }.to_bytes()` passed straight to `encoding_rs::Encoding::for_label(...)`, which implements the WHATWG label-matching algorithm (ASCII-lowercasing, trimming, alias resolution — e.g. the label `\"iso-8859-1\"` deliberately resolves to windows-1252, not genuine ISO/IEC 8859-1, per the Encoding Standard's own Web-compat mandate, not a bug in this plan or in `encoding_rs`)."
    status: pending
  - id: leaf-decode-lossy-and-strict
    content: "`#[no_mangle] pub extern \"C\" fn emerald_rt_encoding_decode(raw: *const c_char, label: *const c_char) -> *mut c_char` — `resolve(label)` (abort via the plan-45 File-I/O-error precedent, `fprintf(stderr, ...); exit(1)`, on an unresolvable label), then `encoding.decode(unsafe { CStr::from_ptr(raw) }.to_bytes())`, which per the WHATWG spec never fails — malformed/unmappable byte sequences become U+FFFD — copy the resulting `Cow<str>`'s bytes into a fresh `emerald_alloc`-obtained, null-terminated buffer (see Decision log) and return it. `emerald_rt_encoding_decode_strict` calls the same `decode()` (which also returns a `had_errors: bool`) and returns `std::ptr::null_mut()` on `had_errors || resolve(label).is_none()` instead of aborting — `sema`/`codegen` type this call's result as `String?`, reusing plan 43/59's already-shipped null-pointer-is-`nil` convergence verbatim, no new nilability plumbing needed."
    status: pending
  - id: leaf-encode
    content: "`emerald_rt_encoding_encode(text: *const c_char, label: *const c_char) -> *mut c_char` — `resolve(label)` (abort on unresolvable, matching `decode`'s policy), `encoding.encode(unsafe { CStr::from_ptr(text) }.to_str().unwrap_or_else(|_| /* lossy path, see Decision log */))`, copy the resulting byte `Cow<[u8]>` (NOT necessarily valid UTF-8 — the whole point) into a fresh `emerald_alloc` buffer with a trailing NUL appended manually (the source bytes may legitimately be non-UTF-8, but must still not themselves contain an embedded NUL for the null-terminated-buffer convention to round-trip safely — disclosed, not silently assumed)."
    status: pending
  - id: leaf-sema-codegen-namespace-arm
    content: "Give `Encoding` the exact `File`-shaped intrinsic arm plan 45 already established (`crates/emerald-sema/src/lib.rs`'s `infer_expr_type`, guarded on `matches!(recv.as_ref(), Expr::Ident(n) if n == \"Encoding\")`, checked before the real `ModuleDef`-backed dispatch arm for the same reason plan 45 gives — `Encoding` is never declared via a real `module ... end`, so it can never actually collide with `classes`/`module_names`) with three fixed signatures: `decode(String, String): String`, `decode_strict(String, String): String?`, `encode(String, String): String`. Mirror in `emerald-codegen`'s `build_method_call` early-return."
    status: pending
  - id: leaf-example-and-rust-tests
    content: "Add `examples/charset_encoding_proof.em` (the Concrete Proof below) to `emerald-cli/tests/examples.rs`'s checked table. Add `#[test]`s in `emerald-rt` asserting `emerald_rt_encoding_encode`/`decode` round-trip for at least windows-1252 and shift_jis against `encoding_rs`'s own documented behavior directly (no Emerald compilation involved in these specific tests — the same `rlib`-side proof style plan 91 established), plus one asserting `decode_strict` returns null for a genuinely non-UTF-8 buffer under the `\"utf-8\"` label."
    status: pending
isProject: false
---

# Plan 153 — Character Set / Encoding Conversion

Plan 45's `String` surface — `.upcase`/`.downcase`/`.strip`/`.split`/`.split_count`/
`.to_i`/`.to_f`/`.length`/`.slice`, `File.read`/`File.write` — and plan 59's own
finding about `String`'s real representation (a bare, `malloc`-backed,
null-terminated buffer, "bit-for-bit identical to C's own `char*`") both take
UTF-8 as a given: `spec/TYPE_SYSTEM.md` §9 declares `String` UTF-8 encoded, and
every existing intrinsic assumes it. Nothing about a file on disk guarantees
that, though — a CSV export from a Windows-1252-locale spreadsheet, a Shift_JIS
log line from a Japanese legacy system, a GBK-encoded record from a database
that predates UTF-8 adoption are all real, common inputs `File.read` (which,
per plan 45, is "backed by real libc file I/O" with no encoding awareness
whatsoever) will happily hand back as a `String` whose bytes are not valid
UTF-8 at all. This plan is the one place in the stdlib that names that gap
directly and gives a program a way to cross it deliberately, in both
directions, using `encoding_rs` — the actual production implementation of the
WHATWG Encoding Standard, not a reimplementation of it.

`encoding_rs` is not a generic-purpose choice picked for convenience: it is,
verified this session against its own `lib.rs`/`docs.rs` listing (version
0.8.41, released 2026-09-09, MSRV 1.88), "used in Gecko starting with Firefox
56" — the actual charset-conversion engine behind one of the two dominant
browser engines, maintained by Henri Sivonen, with 26,019 dependent crates and
roughly 39.6 million downloads a month, the #10-ranked crate in crates.io's
own "Text processing" category. Its design is deliberately narrow and
spec-literal (its own README states plainly: "No Extensibility by Design",
"No Convenience API for Custom Replacements") — it implements exactly the
encodings the Encoding Standard defines as the Web-compatible set, decoding
always succeeds (replacing malformed input with U+FFFD rather than erroring),
and it explicitly does not implement UTF-7 (its own README: "For decoding
character encodings that occur in email, use the `charset` crate instead...
It wraps this crate and adds UTF-7 decoding" — a real, disclosed exclusion
this plan inherits rather than works around).

## Concrete proof this plan targets

```ruby
text: String = "café"
latin1: String = Encoding.encode(text, "windows-1252")
roundtrip: String = Encoding.decode(latin1, "windows-1252")
puts roundtrip

bogus: String? = Encoding.decode_strict(latin1, "utf-8")
bogus ||= "not valid utf-8"
puts bogus

good: String? = Encoding.decode_strict(latin1, "windows-1252")
good ||= "should not happen"
puts good
```

Expected output:
```
café
not valid utf-8
café
```

`text.encode("windows-1252")` produces the real single-byte windows-1252
encoding of "café" — `63 61 66 e9`, where the trailing `e9` is *not* a valid
standalone UTF-8 byte (it is a UTF-8 continuation-range byte with no lead
byte before it). `Encoding.decode(latin1, "windows-1252")` correctly
transcodes those four bytes back to the real UTF-8 encoding of "café" (`63 61
66 c3 a9`, 5 bytes). `Encoding.decode_strict(latin1, "utf-8")` — asking
"is `latin1`'s buffer *already* valid UTF-8?" — must return `nil`, because
`e9` alone is genuinely malformed UTF-8; `||=` (plan 43's real mechanism)
substitutes the fallback string, proving the null-pointer-is-`nil`
convergence plan 59 established applies here too. `Encoding.decode_strict(latin1,
"windows-1252")`, the correct label, succeeds and round-trips exactly.

## Decision log

- **`String` has a documented, but not runtime-enforced, UTF-8 invariant —
  this plan is the first to *knowingly* produce a `String` value that
  violates it, and discloses that openly rather than inventing a new type to
  avoid the question.** Verified this session: nothing in `emerald_runtime.c`
  or `emerald-codegen` validates UTF-8 anywhere — `emerald_alloc` is a bare
  `malloc` wrapper (plan 59's own finding), `File.read` copies file bytes
  verbatim (plan 45), and plan 45's own `.upcase`/`.downcase` are already
  disclosed as "byte-wise ASCII case conversion only" specifically because a
  byte-wise op "can corrupt a multi-byte UTF-8 sequence" — i.e. the compiler
  already tolerates `String` values that aren't really UTF-8, it just never
  had a reason to *produce* one on purpose before. `Encoding.encode`'s return
  value is exactly that: a `String`-typed value holding legacy-encoded bytes
  that are not, in general, valid UTF-8. The one honest way to build this
  feature inside a type system with no separate `Bytes`/`ByteString` type
  (plan 59's real `Type` enum has none) is to reuse `String` for the raw-byte
  carrier and say so loudly, in this document, rather than pretend the value
  is "still a real String" or invent a whole new resource type (plan 93's
  eventual territory, not this plan's) for a narrow interop escape hatch.
  The practical consequence, stated directly: **never call a Unicode-aware
  String method (plan 154's `.nfc`/`.graphemes`/etc., or any future one) on
  an `Encoding.encode` result** — only `Encoding.decode`/`decode_strict` and
  `File.write` (a raw byte sink, per plan 45) are safe consumers of it.
- **`decode` (lossy) and `decode_strict` (nilable) are two separate
  intrinsics, not one function with a flag, because `Boolean` isn't in
  plan 59's real extern-signature allow-list and Emerald has no default-
  argument mechanism to hide a flag behind.** `encoding_rs::Encoding::decode`
  already returns `(Cow<str>, &'static Encoding, bool had_errors)` in one
  call — both Emerald-side intrinsics wrap that *same* underlying
  `encoding_rs` call, they just react to `had_errors` differently (silently
  keep the replacement-character output vs. return `nil`). This is a thin,
  deliberate divergence in policy, not two different algorithms.
- **Unknown/unresolvable labels abort for `decode`/`encode`, but fold into
  `nil` for `decode_strict` — a real, disclosed asymmetry, not an
  inconsistency.** `decode`/`encode` have no nilable return type to express
  "and also, maybe, the label itself was garbage" without conflating it with
  "and also, maybe, the bytes were malformed" — matching plan 45's own
  File-I/O-error-is-a-controlled-abort precedent (`fprintf(stderr, ...);
  exit(1)`) keeps this plan's failure mode identical in *kind* to an
  existing, already-accepted one. `decode_strict` already has a nilable
  return for the malformed-bytes case, so folding "label didn't resolve"
  into that same `nil` costs nothing new and avoids a second, redundant
  abort path for what is, from the caller's perspective, the same "I asked
  for something and didn't get valid text back" outcome.
- **Every `Encoding` function reuses the `emerald_alloc`-callback allocation
  pattern this plan is the first to need, and every later plan in this batch
  that allocates a new `String` from Rust reuses it identically.** `encoding_rs`
  (like every crate in this batch) allocates its own `String`/`Vec<u8>`
  output on Rust's ordinary heap — but Emerald's memory model, per plan 59,
  is "every `String` is an `emerald_alloc`-obtained buffer," with no
  Rust-`Box`-and-hand-a-fat-pointer-back alternative anywhere in the runtime.
  Rather than invent a second allocation regime, `emerald-rt`'s `lib.rs`
  declares `unsafe extern "C" { fn emerald_alloc(size: i64) -> *mut c_char;
  }` — an ordinary Rust FFI declaration resolved at the *final* `cc` link
  step (plan 91's own `link` step already links the C archive and the Rust
  archive together; the symbol needs to exist only then, not when `cargo
  build -p emerald-rt` type-checks the crate in isolation, exactly the same
  two-object-files-referencing-each-other resolution any ordinary multi-TU C
  link already does) — and every Rust function in this crate that produces a
  new `String` calls it, then `ptr::copy_nonoverlapping`s its own computed
  bytes plus a trailing NUL into the returned buffer. This is the first
  domain plan whose Rust archive calls back into the C archive's own
  allocator rather than only being called *by* the C-archive-linked driver —
  a genuinely new direction of dependency plan 91's proof function never
  needed, stated here explicitly rather than left implicit.
- **The WHATWG label list, not a hand-picked Emerald enum, is the real
  input vocabulary — `label` is a plain `String`, not a new `Encoding`
  sema-level type.** `Type::Enum` exists in plan 59's real `Type` enum, and
  a closed `Encoding` enum was considered; it was declined because the
  WHATWG Encoding Standard's registered label table is roughly forty
  canonical encodings with well over one hundred recognized aliases (e.g.
  `"latin1"`, `"iso-8859-1"`, and `"windows-1252"` all resolve to the same
  encoding on purpose), and `encoding_rs::Encoding::for_label` already
  *is* the correct, spec-exact implementation of that whole matching
  algorithm — hand-porting it into a compile-time-checked Emerald enum would
  be strictly worse (a second, drifting copy of a spec table) for no real
  safety gain, since an invalid label is still just a runtime string value a
  type checker can't validate against a live spec table anyway. This matches
  plan 45's own `File.read(path: String)` precedent: a namespace-style
  static call taking a plain string argument, not a bespoke sema type.
- **This plan does not attempt encoding *detection* (sniffing an unlabeled
  byte stream to guess its encoding) — that is a different, statistical
  problem with a different, named crate (`chardetng`, from the same
  `hsivonen` author, explicitly designed to pair with `encoding_rs`), not
  this plan's.** `Encoding.decode`/`decode_strict`/`encode` all require an
  explicit, caller-supplied label. A hypothetical `Encoding.detect(raw):
  String?` is a real, coherent follow-on this plan deliberately leaves
  outside its own scope, rather than blur "converting between two known
  encodings" and "guessing an unknown one" into a single plan.
- **Out of scope.** UTF-7 (excluded by `encoding_rs` itself; use the
  `charset` crate in a future plan if this ever becomes a real need — email
  MIME decoding specifically), a `Bytes`/`ByteString` first-class type (the
  narrower, disclosed reuse of `String` above is this plan's deliberate,
  stated alternative, not a placeholder for a "real" type later), streaming
  decode/encode across buffer boundaries (`encoding_rs`'s `Decoder`/`Encoder`
  streaming API exists and this plan does not wrap it — every function here
  takes one whole `String` and returns one whole `String`, matching every
  other non-streaming intrinsic in this stdlib today), and any change to
  `File.read`/`File.write` themselves (they stay exactly as plan 45 left
  them — raw-byte, encoding-unaware — this plan is the conversion step a
  caller runs before/after them, not a replacement for them).
