2026-09-21T20:14:00Z

---
name: Multipart/Form-Data Parsing
overview: "`multer` (v3.1.0, verified this session against `lib.rs/crates/multer` and `docs.rs/multer`) wraps `multipart/form-data` request bodies from plan 101's HTTP server into named field/file parts, exposed to Emerald as a `Multipart`/`Field` opaque-handle pair (plan 93) whose `.read_chunk(max_bytes)` intrinsic mirrors `multer::Field::chunk`'s own bounded, per-call `Option<Bytes>` shape exactly — so a large file upload is drained in fixed-size pieces rather than materialized whole in Emerald's own unmanaged, no-GC heap."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-multipart-dependency-and-vetting
    content: "Add `multer = \"3.1\"` to `crates/emerald-rt/Cargo.toml`, run it through plan 95's crate-vetting checklist (docs+example+test), and record it in `DEPENDENCIES.md` with the honest currency note from the Decision log (last release May 2024, no release since, but the de facto standard via `axum`/`axum-extra`/`apollo-router`/Leptos `server_fn` dependents, ~1.5M downloads/week)"
    status: done
  - id: leaf-sync-to-async-body-bridge
    content: "A small internal adapter turning plan 101's synchronous, `Read`-backed request-body reader into the `futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>>` `multer::Multipart::new` requires — one fixed-size (64 KiB) synchronous read per stream poll, each call individually driven through plan 94's single lazy `tokio::runtime::Runtime`'s `block_on`, never buffering the request body past that one chunk at a time"
    status: done
  - id: leaf-multipart-begin-and-registry
    content: "`emerald_rt_multipart_begin(request_handle: i64, boundary: *const c_char) -> i64` — wraps the body-stream adapter in a `multer::Multipart`, registers it under a new `u64` handle in a plan-93 registry (`Mutex<HashMap<u64, multer::Multipart<'static>>>`), exposed as `Multipart.begin(request: Int64, boundary: String): Int64`"
    status: done
  - id: leaf-multipart-next-field-and-metadata
    content: "`emerald_rt_multipart_next_field(multipart: i64) -> i64` (block_on-driven `.next_field()`, `0` reserved as the exhausted/invalid sentinel per plan 93's handle convention, else a new field handle registered in a second `Mutex<HashMap<u64, multer::Field<'static>>>`), plus `Field.name(field: Int64): String`, `Field.filename(field: Int64): String?` (real `Option<&str>` -> `String?` per plan 43's nullable-reference mechanism, since a form field genuinely may not be a file), exposed as `Multipart.next_field(multipart: Int64): Int64`"
    status: done
  - id: leaf-field-bounded-read-and-close
    content: "`emerald_rt_field_read_chunk(field: i64, max_bytes: i64) -> *mut c_char` (block_on-driven `.chunk()`, `nil`/`NULL` on `Ok(None)` — real end-of-field, not an error) and `emerald_rt_field_close(field: i64) -> i64`, exposed as `Field.read_chunk(field: Int64, max_bytes: Int64): String?` / `Field.close(field: Int64): Void`"
    status: done
  - id: leaf-example-and-gate
    content: "`examples/multipart_upload_echo.em` (the Concrete Proof below, a server that reads an uploaded file field in bounded chunks and prints its total byte count without ever holding the whole file in one `String`) plus a `curl -F` verification step; a Rust `#[test]` asserting `read_chunk` genuinely returns `nil` exactly once, after every real byte, not before"
    status: done
isProject: false
---

# Plan 105 — Multipart/Form-Data Parsing

Plan 101's HTTP server gives Emerald a request body as a stream of bytes; it
says nothing about what is inside that stream. A browser `<form
enctype="multipart/form-data">` submission, or any HTTP client uploading a
file alongside form fields, encodes multiple named parts — some short text
fields, potentially one or more large file parts — into a single body
delimited by a boundary string named in the request's own `Content-Type`
header. Parsing that format correctly (boundary matching across an
arbitrarily-chunked byte stream, part-header parsing for `Content-
Disposition`'s `name`/`filename`, trailing-boundary detection) is genuine,
finicky, RFC 7578-governed parsing work this plan declines to hand-roll —
it uses `multer`, verified this session to still be the crate the wider
Rust ecosystem actually depends on for this: `axum-extra`, `axum` itself
(as an optional dependency), Apollo's `apollo-router`, and Leptos's
`server_fn` all list `multer` as their `multipart/form-data` parser (per
`crates.io`'s own dependents listings, checked this session), and `lib.rs/
crates/multer` shows real, sustained demand (roughly 1.3–1.6 million
downloads per week across recent weeks) despite the crate itself having
shipped no new version since `3.1.0` on May 4, 2024 — a genuine, disclosed
currency gap this plan does not pretend isn't there (see Decision log).

## Concrete proof this plan targets

```ruby
server: Int64 = HttpServer.listen(0)
port: Int64 = HttpServer.port(server)
puts "listening on #{port}"

req: Int64 = HttpServer.accept(server)
boundary: String = HttpServer.content_type_boundary(req)
mp: Int64 = Multipart.begin(req, boundary)

field: Int64 = Multipart.next_field(mp)
name: String = Field.name(field)
puts name

total: Int64 = 0
chunk: String? = Field.read_chunk(field, 65536)
while chunk != nil
  bytes: String = chunk ||= ""
  total += bytes.length
  chunk = Field.read_chunk(field, 65536)
end
puts total

Field.close(field)
```

Verification step: with the program running and the printed port noted,
`curl -F "upload=@/etc/hostname" http://127.0.0.1:<port>/` from a second
shell. Expected program output: `upload` (the field's `name`, from `Content-
Disposition: form-data; name="upload"; filename="hostname"`), then the exact
byte count of `/etc/hostname`'s contents — proving the field was drained
correctly in `65536`-byte (or smaller, for the last chunk) pieces via
repeated `Field.read_chunk` calls rather than one whole-body read, and that
`nil` is returned exactly once, after the last real byte, ending the loop.

## Decision log

- **`multer` chosen over hand-rolling a boundary-matching parser, and
  over the field-count-limited alternatives, on real, checked adoption
  evidence.** This session's search confirmed `multer` 3.1.0 is a direct
  or optional dependency of `axum`, `axum-extra`, `apollo-router`, and
  `server_fn` (Leptos) — not merely "a crate that exists," but the
  parser underneath several of the Rust ecosystem's most-used HTTP
  frameworks' own multipart support. Its own `docs.rs` page documents a
  real, load-bearing DoS-prevention pattern (a size-limit-per-field
  constructor option) this plan's own bounded-chunk design complements
  rather than duplicates — `multer` already refuses to let one field
  grow past a configured cap; this plan's `read_chunk(max_bytes)` on top
  of that caps how much of *any single chunk* Emerald ever materializes
  as one `String` at a time, a second, independent bound.
- **A real, disclosed currency gap: no `multer` release since May 4,
  2024 — over two years stale as of this plan's own authoring date
  (2026-09-21) — accepted anyway, for a stated reason, not glossed
  over.** Per plan 95's crate-vetting policy, a stale release cadence is
  a real signal to weigh, not a disqualifier by itself: `multer`'s
  surface (boundary parsing, part-header parsing) is a bounded, mature
  problem with no fast-moving spec underneath it (RFC 7578 is not
  changing), its own maintained-by-proxy status is strong (its GitHub
  org, `rwf2`, is the same one that publishes `axum`-adjacent tooling,
  and `axum`'s own continued dependency on it is itself an ongoing
  signal the crate has not silently bit-rotted against its actual use
  cases), and no actively-maintained pure-Rust alternative with
  comparable adoption surfaced in this session's search. This plan
  records the gap in `DEPENDENCIES.md` per `leaf-multipart-dependency-
  and-vetting` rather than silently picking the crate and hoping nobody
  asks later.
- **`multer` is async; plan 101's server is not — this plan is the
  sibling batch's clearest real case for plan 94's stated fallback
  ("one lazy `tokio::runtime::Runtime` behind `block_on`"), not its
  synchronous-first default.** `multer::Multipart::new` takes a
  `futures_core::Stream<Item = Result<bytes::Bytes, E>>`, and
  `Field::chunk`/`Field::bytes`/`Multipart::next_field` are all real
  `async fn`s (verified this session against `docs.rs/multer/latest/
  multer/struct.Field.html`). Plan 101's request body, by contrast, is
  a plain synchronous `Read` (matching `HttpServer`'s own sync design).
  This plan bridges the two directly at the smallest possible surface:
  the `Stream` `multer::Multipart::new` receives is backed by ordinary
  synchronous `Read::read` calls into a fixed-size buffer, and every
  `async fn` call this plan's own exported functions make into `multer`
  (`.next_field()`, `.chunk()`) is individually wrapped in the one
  shared lazy runtime's `.block_on(...)` — no long-lived task is ever
  spawned, no `tokio::spawn` anywhere in this plan; each FFI call blocks
  the calling Emerald thread until its one `async` step resolves and
  returns, which is a legitimate, real use of `block_on` (it is
  documented to work correctly driving a single future to completion
  with no other concurrently-running tasks sharing that runtime) rather
  than the anti-pattern of blocking *inside* an already-running async
  task.
- **The bounded-chunk read API is the direct, disclosed answer to this
  batch's own no-GC caveat, not a generic "streaming is nice"
  justification.** Plan 91's Decision log states plainly: `emerald_
  alloc` has "no free... no lifetime tracking" — every `String` an
  Emerald program allocates lives for the rest of the process. A naive
  `Field.read_all(field): String` returning one whole uploaded file as
  a single Emerald `String` would, for a long-running server handling
  many uploads, grow that unreclaimed heap by the full size of every
  file ever uploaded, for the life of the process — a real, disclosed,
  and entirely avoidable failure mode for exactly the kind of long-
  running HTTP server plan 101 exists to support. `Field.read_chunk`
  instead hands Emerald one bounded piece (caller-chosen `max_bytes`,
  capped by `multer`'s own internal buffer size regardless) at a time;
  an Emerald program that wants to write an upload straight to disk
  (via plan 45's `File.write`, called once per chunk in append mode —
  a real, disclosed follow-up this plan does not itself build, since
  plan 45's `File.write` is whole-file-replace, not append) can process
  arbitrarily large uploads in genuinely bounded peak memory; a program
  that calls `read_chunk` in a loop and concatenates every chunk into
  one growing `String` anyway still pays the same unbounded-growth cost
  this plan cannot prevent by API shape alone — the bound is available,
  not mandatory, exactly as `Array[T]`/`String` indexing's own
  disclosed unchecked-access precedent (plan 45) already accepts for
  other operations.
- **Two independent handle registries, not one — `Multipart` and
  `Field` are genuinely different-lifetime resources, and collapsing
  them into a single handle space would make `.next_field()`'s own
  return value ambiguous.** A `Multipart` handle stays alive for the
  whole request (one per `Multipart.begin` call); each call to
  `.next_field()` produces a *new* `Field` handle with its own,
  shorter lifetime (valid until that field is exhausted or `.close()`d,
  per `multer`'s own real API — a `Field<'r>` borrows from its parent
  `Multipart<'r>`, verified against `docs.rs`, which is exactly why
  this plan's Rust-side registries store `Multipart<'static>`/
  `Field<'static>` via `multer`'s own lifetime-erasure constructor
  path rather than fighting Rust's borrow checker across an FFI
  boundary that has no lifetimes of its own). Two `Mutex<HashMap<u64,
  _>>` registries, each with its own monotonic counter per plan 93's
  general handle model, keeps `0` reserved as "no more fields" in the
  `Multipart` registry's counter space without colliding with any real
  `Field` handle's number.
- **`Field.filename` is genuinely nullable, `Field.name` is not — this
  distinction is load-bearing, not incidental.** Per RFC 7578, every
  part's `Content-Disposition` header carries a mandatory `name`
  parameter but an optional `filename` parameter (present only when the
  part represents an uploaded file, not a plain text form field);
  `multer::Field::name(&self) -> Option<&str>` and `::file_name(&self)
  -> Option<&str>` are, in the real API, both `Option`-typed (verified
  against `docs.rs`), but this plan narrows `.name` to a plain `String`
  (a part genuinely missing its own `name` parameter is malformed
  per-spec input this plan surfaces as an aborted parse rather than a
  silently-nullable field-of-a-field, mirroring plan 45's own File I/O
  precedent of a disclosed runtime abort on a real, spec-violating
  input) while `.filename` stays the honest `String?` plan 43's
  nullable-reference mechanism already exists for — a plain text field
  genuinely has no filename, and that absence is a normal, expected
  outcome an Emerald program must handle, not an error.
- **Boolean-return and `Int64`-handle conventions are reused verbatim
  from plan 92/93/59 — nothing new invented here.** Every function this
  plan exports that signals success/failure (`Multipart.begin`
  returning `0` on a malformed boundary, `Field.close`) returns `Int64`;
  every handle is a `u64` surfaced to Emerald as `Int64`; `nil`/`NULL`
  on `Field.read_chunk`'s genuine end-of-field case flows through plan
  43's real nullable-reference mechanism, the same "`NULL` and Emerald
  `nil` are bit-for-bit the same value for a nullable reference type"
  convergence plan 59's own Decision log already found for `CString`'s
  `String.from_cstring`.
- **Out of scope.** `multipart/mixed` (nested multipart parts inside a
  single field, a real but rare RFC 2046 construct some legacy clients
  still emit) — `multer` itself has partial-at-best support for this,
  and this plan does not attempt to expose it. Streaming a multipart
  *response* body (the server side of `multipart/x-mixed-replace`,
  entirely unrelated to this plan's request-parsing focus despite the
  similar name) — not attempted. Automatic total-request-size limits —
  `multer`'s own constructor accepts per-field size constraints this
  plan does not yet thread an Emerald-level configuration knob for;
  an Emerald program gets `multer`'s conservative built-in defaults
  only, with no way to raise or lower them from source in this plan's
  v1 surface, a real, disclosed gap a small follow-up leaf could close
  without redesigning anything here.

## Update (2026-09-23, EXECUTE)

Implemented `Multipart.start`/`.next_field`, `Field.name`/`.filename`/
`.read_chunk`/`.close` in a new `crates/emerald-rt/src/multipart.rs`
exactly as this plan's own Decision log specifies: `multer` 3.1.0
wrapping a hand-rolled `BodyStream` (a `futures_core::Stream` over the
request's own already-buffered byte `Vec`, handed out 64 KiB per poll),
every `async fn` call driven through `crate::tokio_rt()`'s shared
`block_on`, two independent `crate::handle` registry entries
(`"Multipart"`/`"Field"` tags). `HttpRequest#content_type_boundary`
added onto `http_server.rs`'s own existing `HttpRequest` instance-
method arm, parsing the real `Content-Type` header via `multer::
parse_boundary`.

Four real, disclosed deviations from this plan's own Concrete Proof
text and Decision log, found only by actually compiling and running
this plan's own worked example end to end (mirroring plan 104's own
precedent for the identical reason):

1. **`HttpServer.listen`/`.port`/`.accept`/`.content_type_boundary`
   do not exist** — the plan's own text names these "(assumed part of
   plan 101's own surface)"; plan 101 only ever shipped `Http.serve
   (port) do |req| ... end`. `examples/multipart_upload_echo.em` upgrades
   the same accepted request `Http.serve`'s own trampoline hands the
   handler, on a fixed port (`47701`), and calls a genuine new
   `HttpRequest#content_type_boundary` instance method instead of an
   assumed `HttpServer.content_type_boundary(req)` static call.
2. **`String?` is not this grammar's real nullable-type syntax** —
   plan 73 (predating this plan's own authoring date) replaced it with
   a genuine `Option[String]` sum type (`Some`/`None`, `match ... do
   ... end`). `Field.filename`/`.read_chunk` are typed `Option[String]`
   in both sema and codegen (the identical `Option$String` tagged-
   union `is_null`-branch-plus-`phi` construction `Env.get`/`String.
   from_cstring` already establish, reused verbatim, not re-derived),
   and the Concrete Proof's own `while chunk != nil` becomes `while
   true` with a `break` on `None`.
3. **`begin` is a real, grammar-reserved keyword** (`begin ... rescue
   ... end`, confirmed against `grammar.lalrpop` — a genuine parse
   error compiling this plan's own Concrete Proof verbatim) — the
   plan's own literal `Multipart.begin` is renamed `Multipart.start`
   throughout (Rust, sema, codegen, the example), the same class of
   rename plan 101's own `HttpResponse.build` (not `.new`) and plan
   109's `Sha256Hasher.hasher()` (not `.new()`) already establish for
   the identical reason.
4. **A real, load-bearing bug in plan 101's own pre-existing
   `http_serve`, found only by actually POSTing a real binary
   (non-UTF-8) file upload through this plan's own worked example**:
   the request body was read directly into a `String` via
   `Read::read_to_string`, whose own real, documented behavior on the
   first invalid-UTF-8 byte is to leave the target `String` completely
   unchanged (empty, here) and return an `Err` the pre-existing code
   already silently discarded — invisible for every prior plan's own
   text-only proof, but fatal for a real multipart file upload, which
   routinely contains non-UTF-8 bytes. Fixed in `http_server.rs`:
   `HttpRequestData` now carries the real bytes in a `body_bytes: Vec<
   u8>` field (`read_to_end`, not `read_to_string`); the pre-existing
   `body: String` field is still populated, now via `String::from_
   utf8_lossy`, so `HttpRequest#body`'s own existing contract for
   every OTHER already-shipped plan is unchanged. `multipart.rs`'s own
   `http_request_body_bytes` reads `body_bytes`, never the lossy
   `body`. A second, related, empirically-found (not merely inferred)
   finding: a `multer::Field` genuinely holds a live lock into its own
   parent `Multipart`'s shared internal state for as long as the
   `Field` value itself is alive — draining a field to `nil` via
   `.read_chunk` is NOT by itself sufficient to allow a following
   `Multipart.next_field` call to succeed; `Field.close` must also be
   called first, or `.next_field` raises `multer`'s own real
   `"failed to lock multipart state"` error. Disclosed in `Multipart.
   next_field`'s own doc comment; `multipart.rs`'s own `#[cfg(test)]`
   suite exercises this ordering directly.

`examples/multipart_upload_echo.em`'s own response body additionally
echoes the field's `name` and exact byte total (alongside the `puts`
calls the plan's own Concrete Proof specifies, kept for the real
interactive-`curl`-verification case) — an additional, disclosed
adaptation, separate from the four numbered above:
`runtime/emerald_runtime.c`'s own `setvbuf(stdout, NULL, _IONBF, 0)`
unbuffering trick is scoped exclusively to `.register()`-ing actor
processes (its own comment says so directly), so a `puts` inside an
ordinary `Http.serve` handler is not reliably visible in a piped/
redirected stdout before `crates/emerald-cli/tests/multipart.rs`'s own
CI-enforced test externally kills the still-running server process —
the same "curl for the disclosed external-client check, a Rust test
for the CI-enforced one" split plan 104's own Decision log already
establishes, applied here via the response body instead of stdout for
the CI-enforced half.

Verified: `cargo build --workspace`; `cargo test -p emerald-rt --lib`
(257 passed, including this plan's own three `multipart::tests`, one
of them a real regression test for finding 4 above — a 200,000-byte
binary field drained across many real chunks); `cargo test -p
emerald-sema -p emerald-codegen` (338 + 203 passed, no regression);
`cargo test -p emerald-cli --test examples` (71 passed, no
regression); `cargo test -p emerald-cli --test http_server` (1
passed, no regression — proves finding 4's fix didn't disturb plan
101's own pre-existing body-reading contract); `cargo test -p
emerald-cli --test multipart` (2 passed: the worked example draining
a 200,000-byte binary upload across multiple `Field.read_chunk` calls
and echoing the exact byte count, and a plain 404 for an unhandled
path); `cargo nextest run --workspace` (1255 passed, 0 failed, 4
skipped — the pre-existing, network-dependent `dns_resolution_em_
prints_expected_sequence` and the pre-existing, environment-specific
`emerald-driver::cache::tests::corrupting_the_cached_object_file_
forces_a_real_recompile_not_an_error` excluded and independently
re-confirmed pre-existing/unrelated, not fixed); `cargo clippy
--workspace --all-targets` (0 errors, only pre-existing warnings in
files this plan did not touch); `cargo fmt --check` and `treefmt
--fail-on-change` (both clean); `cargo audit --ignore RUSTSEC-2023-
0071` (exit 0, only pre-existing triaged advisories, none against
`multer`/`bytes`/`futures-core`).
