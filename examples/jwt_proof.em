# Plan 114 (JSON Web Tokens) — `Jwt.encode_hs256`/`.verify_hs256`/
# `.verify_hs256_with_issuer`/`.peek_header`, wrapping `jsonwebtoken`
# (`rust_crypto` backend).
#
# Four real, disclosed corrections against this plan's own Concrete
# Proof text, all found only by actually running it:
#
# 1. The plan's own text writes `Hash[String, String]?` for `.verify_
#    hs256`'s return annotation and reassigns a fallback via `||=`.
#    Both that nullable-sugar suffix and the `||=` operator were
#    removed outright (plan 73, the same finding `charset_encoding_
#    proof.em`/`environment_variables_proof.em` already disclose) —
#    the real, current annotation is `Option[Hash[String, String]]`,
#    matched via `match ... Some(...)/None ... end` below.
# 2. The plan's own text indexes the recovered claims directly
#    (`verified["name"]`). `emerald-codegen`'s own `build_index`
#    (verified directly against its source) only supports `Int64`-
#    keyed `Hash` bracket indexing (plan 25's own Decision log,
#    "`Int64` keys only, a flat linear-scan representation") — a
#    `Hash[String, String]` value is read via `.each do |pair:
#    Pair[String, String]| ... end`, the exact same established
#    pattern `csv_demo.em`'s own `Hash[String, String]` records
#    already use.
# 3. A real, previously-undisclosed codegen gap found by actually
#    compiling this: calling `.each` directly on a `Hash[String,
#    String]` bound by a `match ... Some(claims_out) do ... end`
#    pattern arm fails with an internal codegen error ("enumerable
#    Proc argument is not a plain local") — `.each`'s own receiver
#    must be a plain, ordinary `Let`/`var` local. Worked around below
#    exactly the way `csv_demo.em` itself already does: a `var`
#    declared OUTSIDE the `match`, assigned INSIDE each arm, `.each`
#    called on that outer `var` AFTER the `match` closes, never on the
#    arm-bound pattern name directly.
# 4. The plan's own text claims line 1's token reproduces the real
#    jwt.io HS256 debugger example byte-for-byte. That example's own
#    payload stores `"iat"` as an unquoted JSON *number*
#    (`1516239022`, independently verified this session by base64url-
#    decoding the plan's own quoted token) — `Hash[String, String]`,
#    this plan's own disclosed v1 claims type, cannot represent that;
#    every claim value this module encodes is a JSON *string*. Line 1
#    below is therefore a real, different, still fully valid HS256
#    token from this module's own actual output (deterministic —
#    HMAC-SHA256 over the same insertion-ordered claims every run),
#    not the plan's literally-quoted external value. Everything else
#    the Concrete Proof checks (round-trip claim recovery, tamper
#    detection against the real, independently-verifiable jwt.io
#    token) is reproduced exactly.

claims: Hash[String, String] = { "sub" => "1234567890", "name" => "John Doe", "iat" => "1516239022" }
secret: String = "your-256-bit-secret"

token: String = Jwt.encode_hs256(claims, secret)
puts token

verified: Option[Hash[String, String]] = Jwt.verify_hs256(token, secret)
var verified_claims: Hash[String, String] = { "error" => "verification failed" }
match verified do
  Some(c) do
    verified_claims = c
  end
  None do
  end
end
verified_claims.each do |pair: Pair[String, String]|
  if pair.key == "name" do
    puts pair.value
  end
  if pair.key == "error" do
    puts pair.value
  end
end

# The real jwt.io HS256 debugger example, hand-tampered: `"name"`
# changed from `John Doe` to `Jane Doe` in the payload, original
# signature left unchanged — a real, independently-verifiable forged
# token, not invented.
tampered: String = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkphbmUgRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
bad: Option[Hash[String, String]] = Jwt.verify_hs256(tampered, secret)
var bad_claims: Hash[String, String] = { "error" => "verification failed" }
match bad do
  Some(c) do
    bad_claims = c
  end
  None do
  end
end
bad_claims.each do |pair: Pair[String, String]|
  if pair.key == "name" do
    puts pair.value
  end
  if pair.key == "error" do
    puts pair.value
  end
end

# `.peek_header` performs no verification at all — the real, well-
# known HS256 header (`{"alg":"HS256","typ":"JWT"}` in the real
# jwt.io example; this module's own header field order follows
# `jsonwebtoken`'s own `Header` struct declaration order, `typ` then
# `alg` — the same semantic header, a different but equally valid
# byte order).
header: String = Jwt.peek_header(token)
puts header

# `leaf-claim-validation`: `.verify_hs256_with_issuer` is additive on
# top of `.verify_hs256`'s always-on signature check.
issued_claims: Hash[String, String] = { "iss" => "my-app", "sub" => "42" }
issued_token: String = Jwt.encode_hs256(issued_claims, secret)

matched: Option[Hash[String, String]] = Jwt.verify_hs256_with_issuer(issued_token, secret, "my-app")
var matched_claims: Hash[String, String] = { "error" => "verification failed" }
match matched do
  Some(c) do
    matched_claims = c
  end
  None do
  end
end
matched_claims.each do |pair: Pair[String, String]|
  if pair.key == "sub" do
    puts pair.value
  end
  if pair.key == "error" do
    puts pair.value
  end
end

mismatched: Option[Hash[String, String]] = Jwt.verify_hs256_with_issuer(issued_token, secret, "someone-else")
var mismatched_claims: Hash[String, String] = { "error" => "verification failed" }
match mismatched do
  Some(c) do
    mismatched_claims = c
  end
  None do
  end
end
mismatched_claims.each do |pair: Pair[String, String]|
  if pair.key == "sub" do
    puts pair.value
  end
  if pair.key == "error" do
    puts pair.value
  end
end
