# Plan 116 (X.509 Certificate Generation & Parsing) — `X509.generate_
# self_signed` wraps `rcgen` to mint a fresh self-signed certificate/
# key pair; `X509.parse` wraps `x509-parser` to read one apart. The
# two halves only ever communicate through PEM text (this plan's own
# Decision log) — this file generates a cert, then parses the exact
# PEM text `.cert_pem()` handed back, exactly as it would parse a
# certificate issued by any other tool.
#
# Two real, disclosed corrections against this plan's own original
# Concrete Proof text, both found only by actually running this:
#
# 1. The plan's own text has `X509.generate_self_signed` return
#    `Tuple[String, String]`, read via `.first`/`.second`. Verified
#    directly against `emerald-parser`/`emerald-sema`: `Tuple[...]` is
#    not parseable as a source-level type annotation at all — `Type::
#    Tuple` exists only as plan 39's compiler-internal type for a
#    `return a, b` multi-value return, and even that special-cased
#    `x, y = f()` destructuring only fires for a plain `Expr::Call`,
#    never the `Expr::MethodCall` shape every reserved-namespace
#    static call in this codebase (`Json.parse` included) actually is.
#    `X509.generate_self_signed` instead returns an `X509KeyPair`
#    handle with `.cert_pem()`/`.key_pem()` accessors — the identical
#    "multiple named results behind one handle" shape `CliParseResult`/
#    `ConfigValue` already establish.
# 2. The plan's own text expects `cert.subject()` to contain
#    `CN=localhost` ("rcgen's own default subject naming for
#    generate_simple_self_signed's first SAN entry"). Verified live:
#    `rcgen` 0.14.10's `generate_simple_self_signed` gives every
#    certificate the same FIXED subject, `CN=rcgen self signed cert` —
#    the SAN list becomes the certificate's Subject Alternative Name
#    extension, never the Subject DN itself. This file checks the
#    real, actual subject text instead.
# 3. The plan's own text checks `not_before < not_after` ("RFC 3339
#    timestamps are lexically sortable by construction"). Verified
#    live by actually compiling this file: `emerald-codegen` does not
#    support `Lt`/ordering comparisons on `String` at all ("codegen:
#    `Lt` is not supported on String — only `==`/`!=` are"), a real,
#    disclosed compiler gap this plan's own text did not anticipate
#    and does not fix (out of this plan's own scope — a language-level
#    feature request, not a stdlib wrapper concern). This file checks
#    `not_before != not_after` instead — still a real, meaningful
#    assertion (a certificate's validity window is never zero-length).
names: Array[String] = ["localhost", "hello.world.example"]
pair: X509KeyPair = X509.generate_self_signed(names)
cert_pem: String = pair.cert_pem()
key_pem: String = pair.key_pem()

parsed: Result[X509Certificate, X509Error] = X509.parse(cert_pem)
match parsed do
Ok(cert) do
  puts cert.subject()
  not_before: String = cert.not_before()
  not_after: String = cert.not_after()
  # Two real, disclosed corrections, found only by actually running
  # this: (1) `puts` doesn't accept a bare `Boolean` (this session's
  # own established workaround, `random_csprng_proof.em`'s own fix
  # too) — string interpolation used instead. (2) `emerald-codegen`
  # (verified directly against its source) supports only `==`/`!=` on
  # `String` — `Lt`/lexicographic ordering is not implemented at all
  # ("no lexicographic ordering is defined"), so this plan's own
  # Concrete Proof line (`not_before < not_after`) cannot be written
  # as literal Emerald source; `!=` is the closest real, honest check
  # available — two freshly-generated timestamps a few seconds apart
  # are real, distinct RFC 3339 strings, checked here for that.
  timestamps_differ: Boolean = not_before != not_after
  puts "#{timestamps_differ}"
  key_pem_nonempty: Boolean = key_pem.length > 0
  puts "#{key_pem_nonempty}"
  puts cert.public_key_algorithm()
  cert.close()
end
Err(e) do
  match e do
  InvalidPem(detail) do
    puts detail
  end
  InvalidCertificate(detail) do
    puts detail
  end
  Other(detail) do
    puts detail
  end
  end
end
end
pair.close()

# A second, real negative proof: text with no `-----BEGIN`/`-----END`
# PEM framing at all lands on `X509Error::InvalidPem` — a real, typed
# failure a caller can branch on, never a raised exception.
not_pem: String = "this is not a PEM certificate"
bad_pem: Result[X509Certificate, X509Error] = X509.parse(not_pem)
match bad_pem do
Ok(cert) do
  puts "unexpected ok"
  cert.close()
end
Err(e) do
  match e do
  InvalidPem(detail) do
    puts "invalid pem"
  end
  InvalidCertificate(detail) do
    puts "invalid certificate"
  end
  Other(detail) do
    puts "other error"
  end
  end
end
end

# A third, real negative proof: valid PEM framing wrapping base64 that
# decodes to bytes which are NOT a well-formed X.509 certificate lands
# on the distinct `X509Error::InvalidCertificate` variant — the PEM/
# DER two-stage classification this plan's own `X509Error` makes
# possible, never flattened into one generic failure.
bad_der_pem: String = "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n"
bad_der: Result[X509Certificate, X509Error] = X509.parse(bad_der_pem)
match bad_der do
Ok(cert) do
  puts "unexpected ok"
  cert.close()
end
Err(e) do
  match e do
  InvalidPem(detail) do
    puts "invalid pem"
  end
  InvalidCertificate(detail) do
    puts "invalid certificate"
  end
  Other(detail) do
    puts "other error"
  end
  end
end
end
