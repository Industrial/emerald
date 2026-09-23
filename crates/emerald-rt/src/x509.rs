// Plan 116 (X.509 Certificate Generation & Parsing) — `X509.generate_
// self_signed`/`X509.parse`, wrapping `rcgen` (minting) and
// `x509-parser` (reading) behind two independently-choosable entry
// points that only ever communicate through PEM text, per this plan's
// own Decision log.
//
// Two opaque, plan-93-governed handle types:
//
//   - `X509KeyPair` (`X509.generate_self_signed`'s return value) —
//     the boxed value behind the handle is a plain `(String, String)`
//     of `(cert_pem, key_pem)`, nothing fancier is needed since both
//     halves are already ready-to-use PEM text.
//   - `X509Certificate` (`X509.parse`'s `Ok` payload) — the boxed
//     value is the OWNED, decoded DER `Vec<u8>` only, never a stored
//     `x509_parser::certificate::X509Certificate<'a>` view. That type
//     borrows from the DER buffer it was parsed from (verified
//     directly against the vendored 0.18.1 source — `X509Certificate<
//     'a>`'s own `raw: &'a [u8]` field and every `TbsCertificate<'a>`
//     field beneath it), and Emerald's `crate::handle` registry stores
//     `Box<dyn Any + Send>` values with no lifetime parameter at all —
//     there is no way to box "the DER bytes and a view borrowing them"
//     together in safe Rust without a self-referential-struct crate
//     this project doesn't otherwise depend on. This module resolves
//     it more simply than the plan's own text anticipated: every
//     accessor re-parses the stored DER bytes fresh (via `X509Certif
//     icate::from_der`), reads what it needs, and lets the borrowed
//     view drop before returning — a real, disclosed cost (a second
//     ASN.1 parse per accessor call, on top of the re-formatting cost
//     the plan's own Decision log already discloses for `.subject()`/
//     `.issuer()`), but 100% safe, and the stored bytes are guaranteed
//     parseable (validated once, at `X509.parse` time) so the re-parse
//     can never itself fail.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::ffi::c_void;
use std::os::raw::c_char;
use x509_parser::prelude::*;

const X509_KEYPAIR_TAG: &str = "X509KeyPair";
const X509_CERTIFICATE_TAG: &str = "X509Certificate";

// Plan 195 (Typed Domain Errors): `X509Error`'s own variant tags,
// declaration order, matching `emerald-sema`/`emerald-codegen`'s own
// `x509_error_enum_def` byte-for-byte. Real-checked against the
// vendored `x509-parser` 0.18.1 source (`src/pem.rs`'s own `PEMError`,
// `src/error.rs`'s own `X509Error`) rather than assumed: PEM decoding
// (missing `-----BEGIN`/`-----END`, bad base64) and DER/ASN.1
// certificate decoding are two genuinely separate failure stages with
// two separate upstream error types, so this enum keeps both distinct
// rather than folding one into the other from the start.
//   0 InvalidPem(String)          — x509_parser::pem::PEMError
//   1 InvalidCertificate(String)  — x509_parser::error::X509Error
//   2 Other(String)               — any failure neither stage names
const X509_ERROR_TAG_INVALID_PEM: i32 = 0;
const X509_ERROR_TAG_INVALID_CERTIFICATE: i32 = 1;
const X509_ERROR_TAG_OTHER: i32 = 2;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// Re-parses `id`'s stored DER bytes and runs `f` against the fresh
/// `X509Certificate` view — see this module's own doc comment for why
/// every accessor re-parses rather than caching a borrowed view. The
/// re-parse itself cannot fail: `id`'s bytes were already validated by
/// `x509_parse` before being stored.
fn with_cert<R>(id: i64, f: impl FnOnce(&X509Certificate) -> R) -> Result<R, String> {
  handle_get_mut::<Vec<u8>, R>(id, X509_CERTIFICATE_TAG, |der| {
    let (_, cert) = X509Certificate::from_der(der)
      .expect("emerald-rt/x509: stored DER bytes were already validated by X509.parse");
    f(&cert)
  })
}

/// Formats an `ASN1Time` as RFC 3339 text (`YYYY-MM-DDTHH:MM:SSZ`). A
/// real, disclosed correction against this plan's own text, found only
/// by reading the vendored source directly: `ASN1Time`'s own `Display`
/// impl (`x509-parser` 0.18.1's `src/time.rs`) is NOT RFC 3339 — it
/// produces an asctime-like `"Sep 22 12:34:56 2026 +00:00"` shape.
/// `x509_parser::pem::Pem::parse_x509`'s underlying `time` crate type
/// normalizes every certificate timestamp to UTC (`utc_adjusted_
/// datetime`/`utc_datetime`, both in the same vendored source file),
/// so this formatter's own hardcoded `Z` suffix is correct, not an
/// approximation.
fn format_rfc3339(t: ASN1Time) -> String {
  let dt = t.to_datetime();
  format!(
    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
    dt.year(),
    dt.month() as u8,
    dt.day(),
    dt.hour(),
    dt.minute(),
    dt.second()
  )
}

/// Looks up the OID's short name via `oid_registry`, falling back to
/// the OID's own raw dotted-decimal text when the registry has no
/// entry for it — never a panic or an empty string either way. A real,
/// disclosed correction against this module's own first draft: `oid_
/// registry` 0.8.1 has no free `oid2sn` function at all (verified
/// directly against the vendored source, `src/lib.rs`) — `OidRegistry::
/// get(oid)` returning `Option<&OidEntry>`, then that entry's own
/// `.sn()`, is the crate's real lookup API.
fn algorithm_name(oid: &asn1_rs::Oid) -> String {
  let registry = oid_registry::OidRegistry::default().with_all_crypto();
  match registry.get(oid) {
    Some(entry) => entry.sn().to_string(),
    None => oid.to_string(),
  }
}

/// `X509.generate_self_signed(subject_alt_names: Array[String]):
/// X509KeyPair` — a real, disclosed deviation from this plan's own
/// Concrete Proof, which specified `Tuple[String, String]`: verified
/// directly against `emerald-parser`/`emerald-sema` that `Tuple[...]`
/// is not parseable as a source-level type annotation at all (`Type::
/// Tuple` exists only as the compiler-internal type of a `return a, b`
/// multi-value return, per plan 39, and even THAT special-cased
/// destructuring path only fires for a plain `Expr::Call`, never an
/// `Expr::MethodCall` — the shape every reserved-namespace static call
/// in this codebase, `Json.parse` included, actually is). An opaque
/// `X509KeyPair` handle with `.cert_pem()`/`.key_pem()` accessors is
/// this module's own resolution — the identical `CliParseResult`/
/// `ConfigValue` "multiple named results behind one handle" shape
/// plans 182/183 already establish, not a new pattern.
///
/// Never a `Result`: an empty `subject_alt_names` (or any `rcgen`
/// failure) raises a caught `NativeError` via `catch_and_raise`, per
/// this plan's own text ("an empty subject_alt_names array rejected as
/// a caught error") — this is a caller-programming-error class of
/// failure, not an anticipated, routinely-checked one.
///
/// # Safety
/// `names_array` must point to a real `Array[String]` buffer (`[len:
/// i64][elem: *const c_char]*n`).
pub unsafe fn x509_generate_self_signed(names_array: *const c_void) -> i64 {
  if names_array.is_null() {
    crate::raise_native_error("X509.generate_self_signed: null array pointer");
  }
  let arr = names_array as *const i64;
  let len = *arr;
  if len <= 0 {
    crate::raise_native_error("X509.generate_self_signed: subject_alt_names must not be empty");
  }
  let elems = arr.add(1) as *const *const c_char;
  let mut names: Vec<String> = Vec::with_capacity(len as usize);
  for i in 0..len {
    let ptr = *elems.add(i as usize);
    if ptr.is_null() {
      crate::raise_native_error("X509.generate_self_signed: null string in subject_alt_names");
    }
    match read_str(ptr) {
      Ok(s) => names.push(s.to_string()),
      Err(e) => crate::raise_native_error(&format!("X509.generate_self_signed: {e}")),
    }
  }
  match rcgen::generate_simple_self_signed(names) {
    Ok(rcgen::CertifiedKey { cert, signing_key }) => {
      let cert_pem = cert.pem();
      let key_pem = signing_key.serialize_pem();
      handle_alloc(Box::new((cert_pem, key_pem)), X509_KEYPAIR_TAG)
    }
    Err(e) => crate::raise_native_error(&format!("X509.generate_self_signed: {e}")),
  }
}

/// # Safety
/// `id` must be a handle `x509_generate_self_signed` actually returned.
pub unsafe fn x509_keypair_cert_pem(id: i64) -> *const c_char {
  match handle_get_mut::<(String, String), String>(id, X509_KEYPAIR_TAG, |(cert_pem, _)| {
    cert_pem.clone()
  }) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_generate_self_signed` actually returned.
pub unsafe fn x509_keypair_key_pem(id: i64) -> *const c_char {
  match handle_get_mut::<(String, String), String>(id, X509_KEYPAIR_TAG, |(_, key_pem)| {
    key_pem.clone()
  }) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_generate_self_signed` actually returned.
pub unsafe fn x509_keypair_close(id: i64) {
  handle_close(id);
}

/// `X509.parse(pem: String): Result[X509Certificate, X509Error]` —
/// `x509_parser::pem::parse_x509_pem` first (the PEM/base64-decode
/// stage), then `X509Certificate::from_der` against the decoded bytes
/// (the DER/ASN.1-decode stage) to validate the certificate parses
/// before ever storing it — a malformed PEM and a well-formed-PEM-but-
/// malformed-certificate are two distinguishable `X509Error` variants,
/// never flattened into one.
///
/// # Safety
/// `pem`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn x509_parse(pem: *const c_char) -> *mut c_void {
  let text = match read_str(pem) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(X509_ERROR_TAG_OTHER, &e),
  };
  let (_, pem_obj) = match parse_x509_pem(text.as_bytes()) {
    Ok(v) => v,
    Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
      return crate::emerald_rt_result_err_tagged_str(X509_ERROR_TAG_INVALID_PEM, &e.to_string());
    }
    Err(nom::Err::Incomplete(_)) => {
      return crate::emerald_rt_result_err_tagged_str(
        X509_ERROR_TAG_INVALID_PEM,
        "incomplete PEM data",
      );
    }
  };
  if let Err(e) = X509Certificate::from_der(&pem_obj.contents) {
    let msg = match e {
      nom::Err::Error(e) | nom::Err::Failure(e) => e.to_string(),
      nom::Err::Incomplete(_) => "incomplete certificate data".to_string(),
    };
    return crate::emerald_rt_result_err_tagged_str(X509_ERROR_TAG_INVALID_CERTIFICATE, &msg);
  }
  let id = handle_alloc(Box::new(pem_obj.contents), X509_CERTIFICATE_TAG);
  crate::emerald_rt_result_ok(id)
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_subject(id: i64) -> *const c_char {
  match with_cert(id, |cert| cert.subject().to_string()) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_issuer(id: i64) -> *const c_char {
  match with_cert(id, |cert| cert.issuer().to_string()) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_not_before(id: i64) -> *const c_char {
  match with_cert(id, |cert| format_rfc3339(cert.validity().not_before)) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_not_after(id: i64) -> *const c_char {
  match with_cert(id, |cert| format_rfc3339(cert.validity().not_after)) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_public_key_algorithm(id: i64) -> *const c_char {
  match with_cert(id, |cert| {
    algorithm_name(&cert.public_key().algorithm.algorithm)
  }) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `id` must be a handle `x509_parse` actually returned an `Ok` for.
pub unsafe fn x509_certificate_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  unsafe fn build_names_array(names: &[&str]) -> (*mut c_void, Vec<std::ffi::CString>) {
    let cstrings: Vec<std::ffi::CString> = names
      .iter()
      .map(|s| std::ffi::CString::new(*s).unwrap())
      .collect();
    let ptr = crate::emerald_alloc(8 + 8 * names.len() as i64) as *mut i64;
    *ptr = names.len() as i64;
    let elems = ptr.add(1) as *mut *const c_char;
    for (i, c) in cstrings.iter().enumerate() {
      *elems.add(i) = c.as_ptr();
    }
    (ptr as *mut c_void, cstrings)
  }

  // A real, generator-driven round trip (per this plan's own leaf-
  // rust-tests-and-example): generate a fresh self-signed certificate
  // for `["localhost"]`, parse the PEM this module's own generator
  // just produced back through this module's own parser, and assert
  // real properties of the result — never a canned fixture, since
  // `rcgen`'s own output is randomly keyed and timestamped per call.
  #[test]
  fn generate_then_parse_round_trips_a_real_self_signed_certificate() {
    unsafe {
      let (names_ptr, _keep_alive) = build_names_array(&["localhost"]);
      let keypair_id = x509_generate_self_signed(names_ptr);

      let cert_pem = cstr(x509_keypair_cert_pem(keypair_id));
      let key_pem = cstr(x509_keypair_key_pem(keypair_id));
      assert!(cert_pem.contains("BEGIN CERTIFICATE"));
      assert!(!key_pem.is_empty());

      let cert_pem_c = std::ffi::CString::new(cert_pem).unwrap();
      let result = x509_parse(cert_pem_c.as_ptr()) as *const i64;
      assert_eq!(
        *result, 0,
        "expected Ok from X509.parse on a freshly generated cert"
      );
      let cert_id = *(result.add(1));

      // Real, disclosed correction against this plan's own Concrete
      // Proof (which expected `CN=localhost`, "rcgen's own default
      // subject naming for generate_simple_self_signed's first SAN
      // entry"): verified live, `rcgen` 0.14.10's `generate_simple_
      // self_signed` gives every certificate the same FIXED subject,
      // `CN=rcgen self signed cert` — `localhost` lands only in the
      // certificate's SAN extension (which this module's v1 accessor
      // surface does not expose; `.public_key_algorithm`/`.subject`/
      // `.issuer`/`.not_before`/`.not_after` are this plan's own named
      // accessor list, and a SAN reader is a real, disclosed scope gap
      // this plan does not add). This assertion checks what `rcgen`
      // actually produces, not what the plan's text assumed.
      let subject = cstr(x509_certificate_subject(cert_id));
      assert_eq!(subject, "CN=rcgen self signed cert");

      let not_before = cstr(x509_certificate_not_before(cert_id));
      let not_after = cstr(x509_certificate_not_after(cert_id));
      assert!(
        not_before < not_after,
        "not_before ({not_before}) should sort lexically before not_after ({not_after})"
      );

      let algo = cstr(x509_certificate_public_key_algorithm(cert_id));
      assert!(!algo.is_empty());

      x509_certificate_close(cert_id);
      x509_keypair_close(keypair_id);
    }
  }

  #[test]
  fn parse_rejects_a_malformed_pem_with_the_invalid_pem_tag() {
    unsafe {
      let bad = std::ffi::CString::new("not a pem at all").unwrap();
      let result = x509_parse(bad.as_ptr()) as *const i64;
      assert_eq!(*result, 1, "expected Err");
      let err_block = *(result.add(1)) as *const i64;
      assert_eq!(
        *err_block, X509_ERROR_TAG_INVALID_PEM as i64,
        "a header-less input should be classified InvalidPem"
      );
    }
  }

  #[test]
  fn parse_rejects_a_pem_wrapping_garbage_der_with_the_invalid_certificate_tag() {
    unsafe {
      let bad =
        std::ffi::CString::new("-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n")
          .unwrap();
      let result = x509_parse(bad.as_ptr()) as *const i64;
      assert_eq!(*result, 1, "expected Err");
      let err_block = *(result.add(1)) as *const i64;
      assert_eq!(
        *err_block, X509_ERROR_TAG_INVALID_CERTIFICATE as i64,
        "valid base64/PEM framing wrapping non-certificate DER should be InvalidCertificate"
      );
    }
  }

  // Deliberately NOT tested here: `x509_generate_self_signed`'s own
  // empty-array rejection calls `crate::raise_native_error`, which
  // calls the real `emerald_raise` C entry point directly (a setjmp/
  // longjmp-style non-local exit, never a catchable Rust panic) — this
  // crate's own `#[cfg(test)] mod test_stubs::emerald_raise` stub
  // deliberately `std::process::abort()`s rather than fake a `longjmp`
  // it cannot actually provide (see that stub's own doc comment: "not
  // called by any test"). Verified instead via `examples/x509_proof.em`
  // run through the real CLI, the same real end-to-end path plan 92's
  // own Concrete Proof already establishes for this exact situation.
}
