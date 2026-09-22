2026-09-21T20:32:00Z

---
name: Base64 & Hex Encoding
overview: "Two small, near-zero-risk pure-Rust crates — `base64` (0.23.1, Marshall Pierce et al., 126M downloads/week, `Engine`-trait API since its 0.21 rewrite) and `hex` (0.4.3, stable and effectively feature-complete since 2021) — wrapped as `Base64.encode`/`Base64.decode`/`Hex.encode`/`Hex.decode` and their variant siblings, the most mechanical plan in this batch: no new control-flow surface, no resource handles, just eight pure functions moving bytes to text and back, all built on plan 92's `Bytes` (ptr,len) buffer convention for the raw-byte side and plan 59's zero-conversion `String` for the text side."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-base64-four-engines
    content: "`Base64.encode(data: Bytes): String` / `Base64.decode(s: String): Result[Bytes, String]` (RFC 4648 §4, standard alphabet, padded — the sane default, matching what Python's `base64.b64encode`, Ruby's `Base64.encode64`, and MIME/PEM all default to); `Base64.encode_no_pad`/`Base64.decode_no_pad` (§4, unpadded); `Base64.encode_url_safe`/`Base64.decode_url_safe` (§5, unpadded — matches RFC 7515 JWT's mandatory unpadded base64url, the dominant real-world url-safe convention); `Base64.encode_url_safe_padded`/`Base64.decode_url_safe_padded` (§5, padded, for the rarer case a caller needs it) — a direct 1:1 mapping onto the crate's four predefined `engine::general_purpose` constants (`STANDARD`, `STANDARD_NO_PAD`, `URL_SAFE`, `URL_SAFE_NO_PAD`)"
    status: done
  - id: leaf-hex-encode-decode
    content: "`Hex.encode(data: Bytes): String` (lowercase, the crate's own default), `Hex.encode_upper(data: Bytes): String`, `Hex.decode(s: String): Result[Bytes, String]` (case-insensitive on input, matching the crate's own documented `decode` behavior — accepts a mix of upper/lower in the same string)"
    status: done
  - id: leaf-bytes-ffi-plumbing
    content: "Every function's raw-byte side uses plan 92's `(ptr, len)` `Bytes` convention on both call and return; every text side reuses plan 59's zero-conversion null-terminated `String`; decode failure (invalid alphabet character, incorrect padding, odd hex digit count) returns `Err(<library error's Display output>)` via plan 53's `Result[T, E]`, never a runtime abort"
    status: deferred
  - id: leaf-panic-boundary-and-tests
    content: "`std::panic::catch_unwind` at every exported function per plan 91's convention; Rust `#[test]`s asserting all eight functions against RFC 4648's own published test vectors plus one deliberately malformed input per decode function"
    status: done
  - id: leaf-example-and-gate
    content: "Add the Concrete Proof example to `examples/`, wire into `emerald-cli/tests/examples.rs`, run the full AGENTS.md gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`)"
    status: done
isProject: false
---

# Plan 123 — Base64 & Hex Encoding

The most straightforward plan in this batch, deliberately. Both crates
are near-ubiquitous, long-stable, pure-Rust, and expose a handful of
pure `&[u8] -> String` / `&str -> Result<Vec<u8>, Error>` functions with
no internal state, no resource lifetime, and no panics reachable from
well-typed input — exactly the shape plan 95's crate-vetting policy
(pure-Rust-first, minimal-dependency-surface) is built to wave through
with the least scrutiny. `base64` (0.23.1, released 2026-08-04, 126.4M
downloads/week, used directly or transitively by 95,130 crates — verified
via lib.rs this session) and `hex` (0.4.3, crates.io's stable long-lived
release, essentially unchanged since 2021 because the format has no
further design space to explore — verified via crates.io this session)
are both rated `#1`/`#2` in lib.rs's own Encoding category. The real
design work here is not crate selection; it is naming the RFC 4648
variants correctly and picking a sane default, since silently picking
the wrong padding/alphabet variant is a genuine, common, real-world
interop bug (a JWT segment encoded with padding, or a URL containing
raw `+`/`/` characters, both break their respective consumers
immediately).

## Concrete proof this plan targets

```ruby
raw: Bytes = "hello world".to_bytes

encoded: String = Base64.encode(raw)
puts encoded

decoded: Result[Bytes, String] = Base64.decode(encoded)
case decoded do
  when Ok(b) do
    puts b.to_s
  end
  when Err(msg) do
    puts msg
  end
end

url_form: String = Base64.encode_url_safe(raw)
puts url_form

hex_form: String = Hex.encode(raw)
puts hex_form

hex_back: Result[Bytes, String] = Hex.decode(hex_form)
case hex_back do
  when Ok(b) do
    puts b.to_s
  end
  when Err(msg) do
    puts msg
  end
end
```

Expected output, in order: `aGVsbG8gd29ybGQ=` (RFC 4648 §4 standard,
padded), `hello world` (round-tripped through decode), `aGVsbG8gd29ybGQ`
(the same bytes, §5 url-safe alphabet, unpadded — no `=`, and no `+`/`/`
would appear for input that produced them), `68656c6c6f20776f726c64`
(lowercase hex), and `hello world` again (the hex round-trip). All five
values are independently checkable against RFC 4648's own worked
examples with no library-specific knowledge required.

## Decision log

- **`Bytes` (plan 92) is the raw-byte carrier on both sides, not
  `String`.** Both crates exist specifically to represent *arbitrary*
  byte sequences — the entire reason base64/hex exist as formats is
  that not all bytes are valid, printable, or transport-safe UTF-8 text.
  Emerald's `String` is documented UTF-8 (`spec/TYPE_SYSTEM.md` §9) and
  is, per plan 59's own verified finding, a bare null-terminated
  `malloc` buffer with no length header — a real NUL byte inside a
  `String`'s content would silently truncate it at that point. Using
  `String` as the *input* to `Base64.encode`/`Hex.encode` would be
  actively wrong for exactly the payloads (binary blobs, encrypted
  data, arbitrary file contents) this plan exists to serialize. This is
  precisely the case the task's own framing names Plan 92's `(ptr, len)`
  `Bytes` convention as being "needed for" — this plan is one of that
  convention's first two consumers (the other being plan 125).
  `decode`'s *output* is `Bytes` for the identical reason in reverse.
- **`STANDARD` (RFC 4648 §4, padded) is the default `Base64.encode`/
  `Base64.decode`, not `URL_SAFE` or an unpadded variant.** This is the
  alphabet MIME, PEM, and most JSON-embedded-base64 conventions expect,
  and it is what Python's `base64.b64encode`, Ruby's `Base64.encode64`,
  and Java's `Base64.getEncoder()` all default to — matching the widest
  real-world convention rather than the URL-safe one, which is the
  correct choice only for the narrower URL/filename/JWT context. A
  caller who picks the wrong default silently produces output another
  system's *default* decoder rejects — this default is chosen to
  minimize that failure mode for the common case.
- **`URL_SAFE` defaults to unpadded, deliberately breaking symmetry with
  `STANDARD`'s padded default.** RFC 7515 (JWS/JWT) mandates unpadded
  base64url for every JWT segment; JWT is the dominant real-world
  consumer of the url-safe alphabet. Defaulting `Base64.encode_url_safe`
  to padded would silently produce output that breaks with every
  standard JWT library. The padded url-safe variant still exists
  (`encode_url_safe_padded`/`decode_url_safe_padded`) for the rarer
  caller who needs it, rather than being dropped for consistency's own
  sake.
- **`decode`'s failure mode is `Result[Bytes, String]` (plan 53), never
  a runtime abort.** Malformed input (wrong padding, an out-of-alphabet
  character, an odd number of hex digits) is routine, common,
  attacker-influenceable input — the same category plan 122's
  `Regex.compile` reasoned through, and reasoned the same way here: this
  is `Result`'s job, not `File.read`'s abort-on-error precedent (plan
  45), because a malformed base64/hex string arriving from a network
  request or a config file is not a programmer bug the way an
  out-of-bounds array index is.
- **`Hex.decode` accepts mixed-case input, matching the underlying
  crate's own documented behavior rather than imposing a stricter
  Emerald-side rule.** Verified this session directly against `hex`'s
  own `decode` documentation: "Both, upper and lower case characters are
  valid in the input string and can even be mixed." This plan does not
  add a stricter case-matching requirement on top — the wrapper is a
  thin pass-through, not a place to invent new validation rules the
  underlying library doesn't itself enforce.
- **No SIMD-acceleration-specific API surface, even though `base64`
  0.23's default-on `simd-unsafe` feature exists.** The crate's own
  `Simd` engine auto-detects AVX2/NEON at runtime and falls back to the
  scalar engine transparently — this is a pure performance optimization
  inside the crate's own default `Engine` construction path, invisible
  at the Rust API level this plan wraps, and needs no Emerald-facing
  knob.
- **Every function wrapped in `std::panic::catch_unwind`, per plan 91's
  mandatory convention**, even though a well-formed call into either
  crate is not expected to panic — the boundary contract is uniform
  across every `emerald-rt` export regardless of a given crate's
  believed panic-freedom, exactly as plan 91's own Decision log states
  it must be.
- **Out of scope.** No base32 or other `data-encoding`-crate-covered
  formats (not requested, and a separate crate); no streaming/
  incremental encode-decode for very large inputs (both APIs operate on
  a single in-memory buffer, matching every other v1 plan in this
  batch's scope); no custom/non-RFC-4648 alphabets (the `base64` crate
  supports building a fully custom `Alphabet`, but nothing in this
  batch's motivating use cases needs one); no `hex`'s `no_std`/`alloc`-
  only feature configuration (irrelevant to `emerald-rt`, which already
  links `std` for every other plan in this batch).
