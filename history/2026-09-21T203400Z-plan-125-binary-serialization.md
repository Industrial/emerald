2026-09-21T20:34:00Z

---
name: Binary Serialization (bincode/msgpack)
overview: "Two binary codecs applied to the same dynamic value tree plan 118 already defined for JSON — `bincode` pinned at `=2.0.1` for fast, compact Emerald-to-Emerald persistence and IPC where both ends run this same runtime, and `rmp-serde` 1.3.1 (MessagePack) for interop with other languages/systems expecting a standard, self-describing binary wire format. This plan discloses, rather than hides, a real finding from this session's own verification pass: `bincode` is now formally unmaintained (its final crates.io release, 3.0.0, is a deliberately broken stub published in response to a maintainer harassment incident) — the plan proceeds with the last functional release pinned exactly, names the disclosed risk plainly, and records `postcard` (the maintainer's own suggested successor) as the documented fallback."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-pin-bincode-2-0-1
    content: "`emerald-rt/Cargo.toml` depends on `bincode = \"=2.0.1\"` (exact pin, not a caret range) — the last functional release before the crate's 2025-12-16 unmaintained-stub final publish; document the pin's reason inline in Cargo.toml and in DEPENDENCIES.md (plan 95) rather than silently pinning with no explanation"
    status: pending
  - id: leaf-serializable-value-reuse
    content: "Both codecs operate on plan 118's own `JsonValue` ADT (or its exact renamed equivalent) — a Rust-side `#[derive(bincode::Encode, bincode::Decode, serde::Serialize, serde::Deserialize)]` on the same enum plan 118 already defines, not a new parallel value type — since Emerald has no `#[derive(Serialize)]`-equivalent mechanism for arbitrary user classes, and building one is a distinct, much larger reflection-adjacent feature this plan does not attempt"
    status: pending
  - id: leaf-bincode-functions
    content: "`Bincode.encode(v: JsonValue): Bytes` / `Bincode.decode(data: Bytes): Result[JsonValue, String]`, using `bincode::encode_to_vec`/`bincode::decode_from_slice` with `bincode::config::standard()`"
    status: pending
  - id: leaf-msgpack-functions
    content: "`MessagePack.encode(v: JsonValue): Bytes` / `MessagePack.decode(data: Bytes): Result[JsonValue, String]`, using `rmp_serde::to_vec`/`rmp_serde::from_slice`"
    status: pending
  - id: leaf-bytes-ffi-and-tests
    content: "Both directions use plan 92's `(ptr, len)` `Bytes` convention for the encoded blob; plan 93's disposal model for any Rust-side allocation the caller doesn't already own; `std::panic::catch_unwind` at every export (plan 91); Rust `#[test]`s round-tripping a representative `JsonValue` tree (nested array + object + null + number + string) through each codec"
    status: pending
  - id: leaf-example-and-gate
    content: "Add the Concrete Proof example to `examples/`, wire into `emerald-cli/tests/examples.rs`, run the full AGENTS.md gate"
    status: pending
isProject: false
---

# Plan 125 — Binary Serialization (bincode/msgpack)

Two codecs, two genuinely different jobs, deliberately not blurred
together. `bincode` produces the smallest, fastest encoding by making no
promise of cross-language or cross-version readability — its own
historical design goal is "the same struct on both ends, compiled by the
same Rust toolchain family." `rmp-serde` produces MessagePack, a
self-describing binary format with implementations in essentially every
mainstream language, at some real cost in size and speed relative to
bincode. This plan's task-level framing states the distinction plainly:
bincode for Emerald talking to Emerald (persistence to local disk, IPC
between two Emerald processes, plan 60's distributed-actors wire format
being one very plausible future consumer), MessagePack for Emerald
talking to anything else.

Both operate on raw bytes, never text, so both use plan 92's `(ptr, len)`
`Bytes` buffer convention throughout, the same convention plan 123
(base64/hex) established as this batch's answer to "the payload is not
guaranteed-valid, NUL-free UTF-8."

## A verified finding this plan does not paper over

This session's own crate-verification pass (WebFetch against docs.rs,
required by this batch's own ground rules) turned up something the task
description did not anticipate: **`bincode` is unmaintained as of this
writing.** `bincode`'s crates.io page states it directly, in the
published README of its own final release: "Due to a doxxing and
harassment incident, development on bincode has ceased. No further
releases will be published on crates.io." That final release, `3.0.0`
(published 2025-12-16), is not a real release at all — it is, by its own
README's description, "only this README, as well as a lib.rs containing
only a compiler error," published specifically so `cargo add bincode`
cannot silently resolve to broken code without an explanation attached.
The last real, functional release is `2.0.1` (2025-03-10).

This is handled the same way this batch's own instructions ask plan 128
(HCL) and plan 129 (Arrow) to handle a maintenance-status surprise:
disclosed plainly, not hidden, and not silently swapped for a different
crate without saying so. Three concrete mitigations, not a shrug:

1. **Pin `bincode = "=2.0.1"` exactly**, not a caret range. A caret
   range (`"2.0.1"`, meaning `>=2.0.1, <3.0.0`) would already never
   resolve to the broken `3.0.0` release regardless, since it is a major
   version bump — but an *exact* pin makes the dependency's frozen,
   deliberately-final status visible directly in `Cargo.toml` and in the
   `DEPENDENCIES.md` ledger plan 95 mandates, rather than looking like
   an ordinary floating dependency to a future maintainer who hasn't
   read this plan.
2. **The crate's own README names its intended successor: `postcard`**
   ("similar in spirit and structure to bincode... a bit differently
   flavored"). This plan does not switch to it now — `postcard` targets
   `no_std`/embedded use cases first and has a different feature/API
   shape this plan has not itself verified against `emerald-rt`'s needs
   — but records it as the documented migration path if plan 95's own
   crate-vetting policy, once written, treats "unmaintained with no
   further security fixes possible" as a hard disqualifier rather than
   a disclosed risk a pinned, audited version can carry.
3. **The risk this specific unmaintained status carries is bounded by
   this plan's own stated use case.** `bincode`'s format is intended for
   trusted, same-toolchain-family payloads (Emerald-to-Emerald IPC/
   persistence, per this plan's own framing) — not for decoding
   attacker-controlled bytes from an open network socket, which is
   exactly the scenario a missing-future-CVE-fix would matter most for.
   `MessagePack`/`rmp-serde` — actively maintained, 1.3.1 published
   2026-09-06 — is this plan's own recommended choice for any payload
   that might originate outside Emerald's own trust boundary, which
   further narrows where `bincode`'s unmaintained status is a live
   concern.

## Concrete proof this plan targets

```ruby
value: JsonValue = Obj({
  "name" => Str("emerald"),
  "version" => Num(0.1),
  "tags" => Arr([Str("compiler"), Str("rust")]),
})

packed: Bytes = Bincode.encode(value)
restored: Result[JsonValue, String] = Bincode.decode(packed)
case restored do
  when Ok(v) do
    puts v == value
  end
  when Err(msg) do
    puts msg
  end
end

wire: Bytes = MessagePack.encode(value)
puts wire.length

back: Result[JsonValue, String] = MessagePack.decode(wire)
case back do
  when Ok(v) do
    puts v == value
  end
  when Err(msg) do
    puts msg
  end
end
```

Expected output, in order: `true` (the bincode round-trip reproduces the
original `JsonValue` exactly, `Ok`, not `Err`), a positive `Int64` byte
count for the MessagePack-encoded form (a real, self-describing wire
size, larger than bincode's for the same value since MessagePack carries
type tags bincode omits), and `true` again (the MessagePack round-trip
also reproduces the original exactly).

## Decision log

- **Both codecs serialize `JsonValue` (plan 118's own answer), not
  arbitrary Emerald class instances — the central, load-bearing scoping
  decision of this whole plan.** Emerald has no macro system, no
  `#[derive]`-equivalent, and no runtime reflection (a locked identity
  constraint, restated throughout this batch) — there is no mechanical
  way to generate a serializer for an arbitrary user-declared `class`
  the way Rust's own `#[derive(Serialize)]` does for an arbitrary
  `struct`. What Emerald *does* already have, once plan 118 lands, is
  one specific, compiler-known dynamic value shape with a real Rust-side
  enum definition sitting behind it — and a Rust enum can trivially wear
  both `serde::{Serialize, Deserialize}` and `bincode::{Encode, Decode}`
  derives, since those macros operate on the Rust type, not on anything
  Emerald-specific. This plan reuses that one already-solved value shape
  rather than re-deriving a second "how do dynamic values map to Rust"
  answer, or attempting the much larger general-object-serialization
  feature.
- **General Emerald object serialization remains explicitly out of
  scope, not merely deferred to "later in this plan."** A real design
  for it would need something like a compile-time-generated per-class
  encoder (codegen emitting a fixed field-order `(ptr, len)` shim per
  `class`, closer in shape to plan 126's protobuf-codegen answer than to
  anything in this plan) — a distinct, larger undertaking this plan does
  not fold in as a side effect just because it shares the word
  "serialization."
- **`bincode::config::standard()` is the only configuration exposed —
  no varint/fixed-int, endianness, or byte-limit knobs.** `bincode` 2.x's
  own API surface (`Encode`/`Decode` traits plus a `Configuration`
  builder) supports several wire-format variants; exposing all of them
  as Emerald-facing options multiplies this plan's API surface for a
  distinction almost no caller needs to control — `standard()` is the
  crate's own recommended default.
- **Decode failure is `Result[JsonValue, String]` (plan 53), matching
  every other fallible-parse plan in this batch (122/123/124's identical
  reasoning): malformed or truncated bytes are routine, not-a-programmer-
  bug failure.**
- **`rmp_serde::to_vec` (compact array-based struct encoding) is used
  over `to_vec_named` (map-based, field-name-tagged encoding) for this
  plan's `JsonValue` payload** — `JsonValue` is already a tagged sum type
  with no named struct fields to preserve, so the named variant's extra
  self-description would only inflate size with no round-trip benefit
  for this specific value shape.
- **Out of scope.** General Emerald object/class serialization (above);
  `bincode`'s own struct-derive fast path for user types (same reason);
  MessagePack extension types (`ext` byte sequences — no Emerald type
  this plan's `JsonValue` shape needs one for); schema evolution or
  versioning of any kind; streaming/incremental encode-decode for very
  large payloads (both APIs build one in-memory `Bytes` buffer, matching
  every other v1 plan in this batch); a migration away from `bincode`
  to `postcard` (named above as the documented future path, not
  executed by this plan).
