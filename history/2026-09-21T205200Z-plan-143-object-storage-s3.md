2026-09-21T20:52:00Z

---
name: S3-Compatible Object Storage — A Minimal Hand-Rolled Client, Not the Full AWS SDK
overview: "An `S3` compiler-provided namespace speaking the S3 REST API directly — a minimal client built on whichever HTTP client crate plans 100/106 already wire into `emerald-rt`, signed correctly via the standalone, AWS-maintained `aws-sigv4` crate — rather than depending on the full `aws-sdk-s3` (176K SLoC, its own bundled HTTP/TLS/async-runtime machinery) or `rust-s3` (lighter, but still a second, redundant bundled HTTP stack alongside plan 100/106's). Flagged explicitly as a heavier, optional-tier plan: real SigV4 request signing and real S3 XML response parsing are both genuine, disclosed complexity this plan does not hand-wave away."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vet-three-options
    content: "Record the three-way crate comparison in `emerald-rt`'s own module doc comment: `aws-sdk-s3` (verified this session: crates.io `1.148.0`, Sep 2026, 176K SLoC, 15MB, `awslabs/aws-sdk-rust`, the official, comprehensive, AWS-first SDK — declined here as too heavy a dependency footprint for one stdlib driver); `rust-s3` (verified: `0.37.2`, May 2026, 9K SLoC, MIT, `durch/rust-s3`, explicitly S3-compatible-endpoint-aware — lighter, but still bundles its own HTTP client/TLS/async-runtime stack, duplicating what plan 100/106 already add to this project); `aws-sigv4` (verified: `1.5.3`, Sep 2026, `smithy-lang/smithy-rs`, AWS-maintained, usable standalone without the rest of the SDK) — chosen, paired with plan 100/106's own HTTP client, as the minimal path that adds signing correctness without a second bundled transport stack."
    status: pending
  - id: leaf-client-handle-and-endpoint
    content: "`S3.connect(endpoint: String, region: String, access_key: String, secret_key: String, bucket: String): Int64` bundles an explicit, caller-supplied endpoint URL (never hardcoded to `s3.amazonaws.com`) with the region/credentials/bucket needed for SigV4's signing scope, registered per plan 93's opaque-handle convention. `endpoint` is what makes this genuinely S3-*compatible* — MinIO, Cloudflare R2, Wasabi, Backblaze B2, and real AWS S3 all speak the same signed-REST surface against different hosts."
    status: pending
  - id: leaf-sigv4-signing
    content: "Every request (`put_object`/`get_object`/`delete_object`/`list_objects`) is signed via `aws-sigv4`'s real canonical-request pipeline before being handed to plan 100/106's HTTP client: build the canonical request (method, canonical URI, canonical query string, canonical headers including `x-amz-content-sha256`, signed-headers list, SHA-256 payload hash), the string-to-sign (algorithm, timestamp, credential scope, hash of the canonical request), the derived signing key (a chained HMAC-SHA256 over the secret key through date, region, `s3`, and `aws4_request`), and the final `Authorization` header — all via `aws_sigv4`'s own implementation, never hand-rolled HMAC/SHA-256 code in this plan."
    status: pending
  - id: leaf-object-operations
    content: "`S3.put_object(client: Int64, key: String, body: String): Void` (signed `PUT`), `S3.get_object(client: Int64, key: String): String?` (signed `GET`, `nil` on a `404` response rather than an error — a missing key is an ordinary, expected outcome, matching plan 140's `Redis.get` posture), `S3.delete_object(client: Int64, key: String): Void` (signed `DELETE`). Every non-2xx, non-404 response parses the real S3 XML error body (`<Error><Code>...</Code><Message>...</Message></Error>`) into a plan-93 error carrying S3's own error code, not a bare HTTP status."
    status: pending
  - id: leaf-list-objects-xml
    content: "`S3.list_objects(client: Int64, prefix: String): Hash[Int64, String]` issues a signed `GET ?list-type=2&prefix=...` and parses the real `ListObjectsV2` XML response (still XML, not JSON, as of this plan's authoring) to extract every `<Contents><Key>` element — a new, disclosed dependency on a lightweight XML-parsing crate (e.g. `quick-xml`), genuine additional complexity beyond signing that this plan states plainly rather than treats as a detail of `get_object`'s pattern repeated. Returns `Hash[Int64, String]` for the same `Array[T]`-has-no-runtime-length reason plan 140 chose it for Redis list replies."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/s3_object.em` (the Concrete Proof below) to `examples/` and the `emerald-cli/tests/examples.rs` table, gated behind plan 92's infra-dependent-example convention. Add `#[test]`s in `emerald-rt`: SigV4 canonical-request/signature construction and XML-parsing logic are tested with fixed, hand-verifiable inputs and no network at all; round-trip `put`/`get`/`delete`/`list` tests run against a real S3-compatible server via `testcontainers` (the `testcontainers-modules` `minio` image — MinIO, not real AWS S3, is this plan's integration-test target, since it speaks the identical signed-REST/XML surface without needing real AWS credentials in CI), `#[ignore]`d or feature-gated where Docker is unavailable."
    status: pending
isProject: false
---

# Plan 143 — S3-Compatible Object Storage

This plan is explicitly heavier than the rest of this batch, and is
flagged that way on purpose: object storage needs correct request
signing (a real cryptographic protocol, not a header a caller can get
approximately right) and a real HTTP transport, both genuine engineering
surfaces rather than a thin wrapper over an existing pure-Rust protocol
implementation the way plans 138-140 got to be. Three real options were
weighed, not one assumed by default. `aws-sdk-s3` (verified this
session: `1.148.0`, Sep 2026, 176K SLoC, 15MB) is AWS's own official,
comprehensive SDK — correct, actively maintained (a release five days
before this plan's authoring), but it brings its own credential-chain
resolution, retry/middleware stack, and HTTP transport (`aws-smithy-
runtime`), none of which this project wants a second copy of once plans
100/106 already give `emerald-rt` an HTTP client. `rust-s3` (verified:
`0.37.2`, May 2026, 9K SLoC, MIT, explicitly S3-compatible-endpoint-
aware per its own README's MinIO/Wasabi/R2/GCP keyword list) is far
lighter, but still bundles its own HTTP client and TLS stack internally
— a second, redundant transport layer, smaller than `aws-sdk-s3`'s but
still duplicated machinery. This plan takes a third path: a minimal,
hand-rolled client speaking S3's REST API directly, built on plan
100/106's own HTTP client for transport and `aws-sigv4` (verified:
`1.5.3`, Sep 2026, maintained by AWS's `smithy-lang` organization,
explicitly usable standalone without the rest of the SDK) purely for
correct request signing — the smallest dependency footprint that still
gets signing right by construction rather than by hand-rolled HMAC code.

## Concrete proof this plan targets

```ruby
client: Int64 = S3.connect("http://localhost:9000", "us-east-1", "minioadmin", "minioadmin", "demo-bucket")
S3.put_object(client, "hello.txt", "hello from emerald")

body: String? = S3.get_object(client, "hello.txt")
body ||= "not found"
puts body

missing: String? = S3.get_object(client, "does-not-exist.txt")
missing ||= "not found"
puts missing

keys: Hash[Int64, String] = S3.list_objects(client, "")
puts keys[0]

S3.delete_object(client, "hello.txt")
```

Expected output: `hello from emerald`, `not found`, `hello.txt` — an
object written and read back through a correctly SigV4-signed `PUT`/
`GET` pair, a real miss defaulted through plan 43's `||=`, and a listed
key extracted from a real parsed XML response. Running this for real
needs a reachable S3-compatible endpoint (MinIO in the example above);
see the testing leaf for how CI provides one.

## Decision log

- **This is a heavier, optional-tier plan, stated as such rather than
  presented at the same weight as plans 137-142.** Every SQL/KV/cache
  driver plan in this batch wraps an existing, complete client
  implementation; this plan builds a real (if narrow) piece of protocol
  logic — request canonicalization, signing, and XML response parsing —
  because the two off-the-shelf alternatives both cost more in
  dependency weight than this plan's own hand-rolled surface does.
- **SigV4 signing is real, disclosed complexity, delegated to a
  verified, correct library rather than hand-rolled.** The pipeline —
  canonical request construction, a SHA-256 hash of the (possibly empty)
  payload, a string-to-sign combining the algorithm/timestamp/
  credential-scope/canonical-request-hash, a signing key derived through
  four chained HMAC-SHA256 operations (date, region, service, literal
  `aws4_request`), and a final HMAC-SHA256 producing the `Authorization`
  header — is exactly the kind of security-sensitive protocol logic this
  project has no business reimplementing by hand when `aws-sigv4` (AWS's
  own maintained implementation, used internally by the full SDK this
  plan otherwise declines) is available standalone. This plan's own code
  is the *plumbing* around `aws-sigv4` (assembling the request, calling
  the signer, attaching headers) — not a reimplementation of the
  cryptographic protocol itself.
- **The endpoint is always caller-supplied, never hardcoded — the
  concrete mechanism that makes this plan "S3-compatible" rather than
  "AWS S3 only."** `S3.connect`'s `endpoint` parameter is an arbitrary
  URL; MinIO, Cloudflare R2, Wasabi, and Backblaze B2 all implement the
  same signed-REST-plus-XML surface this plan speaks, differing only in
  the host they're reached at and, in some cases, the region string
  they expect in the signing scope (also caller-supplied). Nothing in
  this plan's signing or request logic assumes an `amazonaws.com` host.
- **List responses are still XML, not JSON, as of this plan's authoring
  — a second, independent piece of real complexity beyond signing,
  stated separately rather than folded silently into `get_object`'s
  pattern.** S3's `ListObjectsV2` API (and every S3-compatible
  implementation's equivalent) returns an XML document; this plan adds a
  lightweight XML-parsing dependency (`quick-xml`, chosen for being a
  narrow, focused parser rather than a full DOM/serde-XML framework this
  plan's one `<Contents><Key>`-extraction need doesn't require) purely
  to walk that response, a real new dependency this plan discloses
  rather than treats as incidental.
- **String-only object bodies are this batch's most consequential
  instance of the missing-`Bytes`-type gap, not merely a repeated
  footnote.** Every other plan in this batch (137-142) hit the same
  "Emerald's `Type` enum has no byte-buffer type" wall for an edge case
  — a `BLOB` column, a binary cache value. Object storage's single most
  common real workload is binary payloads (images, archives, arbitrary
  files) — `put_object`/`get_object` requiring valid UTF-8 `String`
  bodies is a genuinely significant limitation for this plan
  specifically, not a corner this plan can wave off the way plan 137
  waved off `BLOB` columns as rare. This plan ships that limitation
  anyway, stated plainly, rather than inventing a one-off `Bytes` type
  as a side effect just for this plan — the same cross-cutting-feature
  boundary every other plan in this batch already respected.
- **A missing object is `nil`, not an error — consistent with plan
  140's `Redis.get` and plan 137/138/139's nilable column reads.**
  `get_object` treats a `404 NoSuchKey` response as an ordinary, nilable
  miss (`String?`via plan 43); every other non-2xx status parses the
  real S3 XML error body into a plan-93 error carrying S3's own error
  code (`AccessDenied`, `NoSuchBucket`, `SignatureDoesNotMatch` — the
  last of which is exactly the failure mode a signing bug in this plan's
  own code would produce, making it a meaningful test target for
  `leaf-example-and-tests`).
- **Out of scope.** Multipart upload (S3's real, separate protocol for
  objects too large to `PUT` in one request — a substantial feature in
  its own right, not attempted here), presigned URLs, bucket-level
  operations (create/delete/list buckets, versioning, lifecycle rules,
  ACLs/bucket policies), and streaming upload/download (`put_object`/
  `get_object` fully buffer their body as one owned `String`, matching
  this batch's other buffered-result conventions rather than exposing a
  true streaming body). Connection pooling and credential-chain
  resolution (environment variables, instance metadata, config files —
  `aws-sdk-s3`'s own substantial feature) are also declined; credentials
  are always the two strings `S3.connect` is given directly.
