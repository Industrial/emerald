//! Plan 114 (JSON Web Tokens) — `Jwt.encode_hs256`/`.verify_hs256`,
//! `.encode_rs256`/`.verify_rs256`, `.encode_es256`/`.verify_es256`,
//! `.peek_header`, wrapping `jsonwebtoken` (the `rust_crypto` backend
//! feature — see `Cargo.toml`'s own comment on this dependency for
//! why). Every `verify_*` function requires a secret/public key as a
//! mandatory argument — there is no `Jwt.decode` entry point anywhere
//! in this module that returns claims without a key also having been
//! supplied and checked. The crate's own `Algorithm` enum additionally
//! has no `none`/unsecured variant a `"alg":"none"` header could parse
//! into at all (verified directly against the vendored
//! `algorithms.rs`) — the two defenses are independent and stacked,
//! per this plan's own Decision log.
//!
//! Claims are `Hash[String, String]`, a real, disclosed v1
//! simplification (this plan's own Decision log) — every claim value
//! serializes as a JSON string on encode, never a JSON number/bool/
//! null, even for well-known numeric claims like `iat`/`exp`.
//!
//! A real, disclosed correction against this plan's own leaf text,
//! found only by actually vendoring and reading `validation.rs`:
//! `jsonwebtoken::Validation::new`'s own real default REQUIRES an
//! `exp` claim to be present at all (`required_spec_claims` defaults
//! to `{"exp"}`) — but this plan's own Concrete Proof is the real
//! jwt.io HS256 debugger example, whose claims have no `exp` at all
//! (independently verified this session by base64url-decoding the
//! plan's own quoted token). Requiring `exp` here would make the
//! plan's own worked proof fail to verify. `verify_with_key` below
//! clears `required_spec_claims` of that default entry while leaving
//! `validate_exp` at the crate's own default (`true`) — "always
//! validates `exp`" (this plan's own leaf text) now means "never
//! skipped when present", not "mandatory on every token"; a token that
//! DOES carry a past `exp` is still rejected.
//!
//! `verify_*_with_issuer` is this plan's own `leaf-claim-validation`
//! entry point, adapted to a real, verified language constraint:
//! Emerald has no keyword-argument call syntax at all (checked against
//! `spec/GRAMMAR.md` directly — nothing resembling `issuer: "my-app"`
//! as a call-site argument shape exists anywhere in this codebase), so
//! the plan's own literal `Jwt.verify_hs256(token, secret, issuer:
//! "my-app")` cannot be written as shown without a new, cross-cutting
//! parser feature far outside this leaf's own scope. A separate,
//! explicitly-named function with a plain positional `String`
//! parameter is the narrowest adaptation that keeps the base
//! `verify_*` functions' own arity unchanged (required by the
//! Concrete Proof's own 2-argument calls) while still delivering the
//! Decision log's real functional intent: additive validation layered
//! on top of the always-on signature/`exp` checks, via the crate's own
//! real `Validation.iss`/`.set_issuer`, never a second verification
//! mechanism of this module's own.

use std::ffi::c_void;
use std::os::raw::c_char;

const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// Reads a `Hash[String, String]`'s own real `[pair-count: i64]
/// [(key_ptr, value_ptr), ...]` layout — `csv.rs`'s own `alloc_string_
/// hash`, `emerald-codegen`'s own `build_hash_lit`/`build_hash_map`,
/// reused in shape, not code. Insertion order is preserved, since this
/// runtime layout is a flat array, never a real hash table — reading
/// it in order and serializing that same order (`claims_object` below)
/// is what lets a caller's own claim-declaration order survive into
/// the encoded JWT's payload.
///
/// # Safety
/// `ptr`, if non-null, must point to a real `Hash[String, String]`
/// buffer.
unsafe fn read_string_hash(ptr: *const c_void) -> Result<Vec<(String, String)>, String> {
  if ptr.is_null() {
    return Err("null Hash pointer".to_string());
  }
  let base = ptr as *const i64;
  let count = *base;
  if count < 0 {
    return Err("corrupt Hash: negative pair count".to_string());
  }
  let slots = base.add(1) as *const *const c_char;
  let mut pairs = Vec::with_capacity(count as usize);
  for i in 0..count {
    let k = read_str(*slots.add((i * 2) as usize))?.to_string();
    let v = read_str(*slots.add((i * 2 + 1) as usize))?.to_string();
    pairs.push((k, v));
  }
  Ok(pairs)
}

/// The inverse of `read_string_hash` — allocates a real `Hash[String,
/// String]` value in the identical layout, insertion order preserved.
unsafe fn alloc_string_hash(pairs: &[(String, String)]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 16 * pairs.len() as i64) as *mut i64;
  *ptr = pairs.len() as i64;
  let slots = ptr.add(1) as *mut *mut c_void;
  for (i, (k, v)) in pairs.iter().enumerate() {
    *slots.add(i * 2) = crate::alloc_and_copy_str(k) as *mut c_void;
    *slots.add(i * 2 + 1) = crate::alloc_and_copy_str(v) as *mut c_void;
  }
  ptr as *mut c_void
}

/// `Option[Hash[String, String]]`'s own `[tag: i64][payload: 8]`
/// layout (`regex.rs`'s own `alloc_option_string` — `Some` = 0, `None`
/// = 1, plan 73's fixed encoding).
unsafe fn alloc_option_hash(pairs: Option<&[(String, String)]>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match pairs {
    Some(pairs) => {
      *ptr = OPTION_SOME;
      *(ptr.add(1) as *mut *mut c_void) = alloc_string_hash(pairs);
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

/// Builds the JSON claims object `jsonwebtoken::encode` signs —
/// insertion-order preserved (`serde_json`'s own `preserve_order`
/// feature, already enabled workspace-wide by plan 118's row in this
/// crate's own `Cargo.toml`), every value a JSON string, matching
/// `Hash[String, String]`'s own real type exactly.
fn claims_object(pairs: &[(String, String)]) -> serde_json::Value {
  let mut map = serde_json::Map::new();
  for (k, v) in pairs {
    map.insert(k.clone(), serde_json::Value::String(v.clone()));
  }
  serde_json::Value::Object(map)
}

/// Every decoded claim value crosses back into `Hash[String, String]`
/// as a plain string, regardless of its own real JSON type — this
/// plan's own disclosed v1 simplification. A JSON string keeps its own
/// text verbatim; every other JSON type (number/bool/null/array/
/// object) uses its own compact JSON text as the string value, so a
/// real-world token with a numeric `exp`/`iat` (unlike this plan's own
/// `Hash[String, String]`-produced tokens) still round-trips into
/// something readable rather than silently dropping the claim.
fn claims_to_pairs(claims: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
  claims
    .iter()
    .map(|(k, v)| {
      let s = match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
      };
      (k.clone(), s)
    })
    .collect()
}

/// Shared `decode` + `Validation` construction for every `verify_*`
/// entry point below — see this module's own doc comment for why
/// `required_spec_claims` is cleared of the crate's own default
/// `{"exp"}` entry. Every failure (bad signature, malformed token, an
/// unparseable header, an issuer mismatch) collapses to `None` here —
/// the one point every `verify_*`/`verify_*_with_issuer` function
/// funnels through, so that collapse only needs stating once.
fn verify_with_key(
  token: &str,
  key: &jsonwebtoken::DecodingKey,
  alg: jsonwebtoken::Algorithm,
  issuer: Option<&str>,
) -> Option<Vec<(String, String)>> {
  let mut validation = jsonwebtoken::Validation::new(alg);
  validation.required_spec_claims.clear();
  if let Some(iss) = issuer {
    validation.set_issuer(&[iss]);
    validation.required_spec_claims.insert("iss".to_string());
  }
  match jsonwebtoken::decode::<serde_json::Map<String, serde_json::Value>>(token, key, &validation)
  {
    Ok(data) => Some(claims_to_pairs(&data.claims)),
    Err(_) => None,
  }
}

// ---- HS256 ---------------------------------------------------------

/// `Jwt.encode_hs256(claims: Hash[String, String], secret: String):
/// String`.
///
/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `secret`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn jwt_encode_hs256(claims: *const c_void, secret: *const c_char) -> *const c_char {
  let pairs = match read_string_hash(claims) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_hs256: {e}")),
  };
  let secret = match read_str(secret) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_hs256: {e}")),
  };
  let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
  let key = jsonwebtoken::EncodingKey::from_secret(secret.as_bytes());
  match jsonwebtoken::encode(&header, &claims_object(&pairs), &key) {
    Ok(token) => crate::alloc_and_copy_str(&token),
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_hs256: {e}")),
  }
}

/// `Jwt.verify_hs256(token: String, secret: String): Hash[String,
/// String]?` — every failure mode (bad signature, malformed token, a
/// `"alg":"none"` header the crate's own `Algorithm` enum cannot even
/// parse) collapses to `nil`, never a distinguishable error (this
/// plan's own Decision log, mirroring plan 112's `Password.verify`).
///
/// # Safety
/// `token`/`secret`, if non-null, must point to valid, NUL-terminated
/// C strings.
pub unsafe fn jwt_verify_hs256(token: *const c_char, secret: *const c_char) -> *mut c_void {
  let (token, secret) = match (read_str(token), read_str(secret)) {
    (Ok(t), Ok(s)) => (t, s),
    _ => return alloc_option_hash(None),
  };
  let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::HS256, None);
  alloc_option_hash(result.as_deref())
}

/// `Jwt.verify_hs256_with_issuer(token: String, secret: String,
/// issuer: String): Hash[String, String]?` — this plan's own `leaf-
/// claim-validation`, additive on top of `.verify_hs256`'s always-on
/// signature/`exp` checks (see this module's own doc comment for why
/// this is a separate, positionally-argued function rather than the
/// plan's own literal keyword-argument syntax).
///
/// # Safety
/// `token`/`secret`/`issuer`, if non-null, must point to valid,
/// NUL-terminated C strings.
pub unsafe fn jwt_verify_hs256_with_issuer(
  token: *const c_char,
  secret: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  let (token, secret, issuer) = match (read_str(token), read_str(secret), read_str(issuer)) {
    (Ok(t), Ok(s), Ok(i)) => (t, s, i),
    _ => return alloc_option_hash(None),
  };
  let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::HS256, Some(issuer));
  alloc_option_hash(result.as_deref())
}

// ---- RS256 -----------------------------------------------------------

/// `Jwt.encode_rs256(claims: Hash[String, String], private_key_pem:
/// String): String` — `private_key_pem` is a plain PEM `String`,
/// deliberately not yet a shared `PrivateKey` type (this plan's own
/// Decision log — a future caller with a real key type would just
/// pass `key.to_pem()` here, no signature change needed).
///
/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `private_key_pem`, if non-null, must point to a valid,
/// NUL-terminated C string.
pub unsafe fn jwt_encode_rs256(
  claims: *const c_void,
  private_key_pem: *const c_char,
) -> *const c_char {
  let pairs = match read_string_hash(claims) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_rs256: {e}")),
  };
  let pem = match read_str(private_key_pem) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_rs256: {e}")),
  };
  let key = match jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_rs256: invalid PEM key: {e}")),
  };
  let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
  match jsonwebtoken::encode(&header, &claims_object(&pairs), &key) {
    Ok(token) => crate::alloc_and_copy_str(&token),
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_rs256: {e}")),
  }
}

/// `Jwt.verify_rs256(token: String, public_key_pem: String):
/// Hash[String, String]?` — a malformed `public_key_pem` collapses to
/// `nil`, the same as every other verification failure this module
/// never distinguishes (this plan's own Decision log).
///
/// # Safety
/// `token`/`public_key_pem`, if non-null, must point to valid,
/// NUL-terminated C strings.
pub unsafe fn jwt_verify_rs256(token: *const c_char, public_key_pem: *const c_char) -> *mut c_void {
  let (token, pem) = match (read_str(token), read_str(public_key_pem)) {
    (Ok(t), Ok(p)) => (t, p),
    _ => return alloc_option_hash(None),
  };
  let key = match jsonwebtoken::DecodingKey::from_rsa_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(_) => return alloc_option_hash(None),
  };
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::RS256, None);
  alloc_option_hash(result.as_deref())
}

/// `Jwt.verify_rs256_with_issuer(token: String, public_key_pem:
/// String, issuer: String): Hash[String, String]?` — see `.verify_
/// hs256_with_issuer`'s own doc comment.
///
/// # Safety
/// `token`/`public_key_pem`/`issuer`, if non-null, must point to
/// valid, NUL-terminated C strings.
pub unsafe fn jwt_verify_rs256_with_issuer(
  token: *const c_char,
  public_key_pem: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  let (token, pem, issuer) = match (read_str(token), read_str(public_key_pem), read_str(issuer)) {
    (Ok(t), Ok(p), Ok(i)) => (t, p, i),
    _ => return alloc_option_hash(None),
  };
  let key = match jsonwebtoken::DecodingKey::from_rsa_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(_) => return alloc_option_hash(None),
  };
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::RS256, Some(issuer));
  alloc_option_hash(result.as_deref())
}

// ---- ES256 -------------------------------------------------------------

/// `Jwt.encode_es256(claims: Hash[String, String], private_key_pem:
/// String): String`.
///
/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `private_key_pem`, if non-null, must point to a valid,
/// NUL-terminated C string.
pub unsafe fn jwt_encode_es256(
  claims: *const c_void,
  private_key_pem: *const c_char,
) -> *const c_char {
  let pairs = match read_string_hash(claims) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_es256: {e}")),
  };
  let pem = match read_str(private_key_pem) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_es256: {e}")),
  };
  let key = match jsonwebtoken::EncodingKey::from_ec_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_es256: invalid PEM key: {e}")),
  };
  let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
  match jsonwebtoken::encode(&header, &claims_object(&pairs), &key) {
    Ok(token) => crate::alloc_and_copy_str(&token),
    Err(e) => crate::raise_native_error(&format!("Jwt.encode_es256: {e}")),
  }
}

/// `Jwt.verify_es256(token: String, public_key_pem: String):
/// Hash[String, String]?`.
///
/// # Safety
/// `token`/`public_key_pem`, if non-null, must point to valid,
/// NUL-terminated C strings.
pub unsafe fn jwt_verify_es256(token: *const c_char, public_key_pem: *const c_char) -> *mut c_void {
  let (token, pem) = match (read_str(token), read_str(public_key_pem)) {
    (Ok(t), Ok(p)) => (t, p),
    _ => return alloc_option_hash(None),
  };
  let key = match jsonwebtoken::DecodingKey::from_ec_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(_) => return alloc_option_hash(None),
  };
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::ES256, None);
  alloc_option_hash(result.as_deref())
}

/// `Jwt.verify_es256_with_issuer(token: String, public_key_pem:
/// String, issuer: String): Hash[String, String]?` — see `.verify_
/// hs256_with_issuer`'s own doc comment.
///
/// # Safety
/// `token`/`public_key_pem`/`issuer`, if non-null, must point to
/// valid, NUL-terminated C strings.
pub unsafe fn jwt_verify_es256_with_issuer(
  token: *const c_char,
  public_key_pem: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  let (token, pem, issuer) = match (read_str(token), read_str(public_key_pem), read_str(issuer)) {
    (Ok(t), Ok(p), Ok(i)) => (t, p, i),
    _ => return alloc_option_hash(None),
  };
  let key = match jsonwebtoken::DecodingKey::from_ec_pem(pem.as_bytes()) {
    Ok(k) => k,
    Err(_) => return alloc_option_hash(None),
  };
  let result = verify_with_key(token, &key, jsonwebtoken::Algorithm::ES256, Some(issuer));
  alloc_option_hash(result.as_deref())
}

// ---- header inspection -------------------------------------------------

/// `Jwt.peek_header(token: String): String` — wraps the crate's own
/// `decode_header` (its own explicitly-named, no-verification header
/// inspector). Named `peek_`, not `decode_`, deliberately — this
/// plan's own Decision log — so `Jwt.peek_header(token)["role"]` reads,
/// at the call site, as obviously wrong: this function performs NO
/// signature check and its return value must never be used to make an
/// authorization decision.
///
/// # Safety
/// `token`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn jwt_peek_header(token: *const c_char) -> *const c_char {
  let token = match read_str(token) {
    Ok(t) => t,
    Err(e) => crate::raise_native_error(&format!("Jwt.peek_header: {e}")),
  };
  match jsonwebtoken::decode_header(token) {
    Ok(header) => {
      let json = serde_json::to_string(&header).unwrap_or_else(|_| "{}".to_string());
      crate::alloc_and_copy_str(&json)
    }
    Err(e) => crate::raise_native_error(&format!("Jwt.peek_header: {e}")),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn alloc_test_hash(pairs: &[(&str, &str)]) -> *mut c_void {
    let owned: Vec<(String, String)> = pairs
      .iter()
      .map(|(k, v)| (k.to_string(), v.to_string()))
      .collect();
    unsafe { alloc_string_hash(&owned) }
  }

  unsafe fn read_result_hash(ptr: *mut c_void) -> Option<Vec<(String, String)>> {
    let opt = ptr as *const i64;
    if *opt == OPTION_NONE {
      return None;
    }
    let hash_ptr = *(opt.add(1)) as *const i64;
    let count = *hash_ptr;
    let slots = hash_ptr.add(1) as *const *const c_char;
    let mut pairs = Vec::with_capacity(count as usize);
    for i in 0..count {
      let k = std::ffi::CStr::from_ptr(*slots.add((i * 2) as usize))
        .to_str()
        .unwrap()
        .to_string();
      let v = std::ffi::CStr::from_ptr(*slots.add((i * 2 + 1) as usize))
        .to_str()
        .unwrap()
        .to_string();
      pairs.push((k, v));
    }
    Some(pairs)
  }

  fn find<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
      .iter()
      .find(|(k, _)| k == key)
      .map(|(_, v)| v.as_str())
  }

  #[test]
  fn hs256_round_trip_recovers_the_original_claims() {
    unsafe {
      let claims = alloc_test_hash(&[("sub", "1234567890"), ("name", "John Doe")]);
      let secret = std::ffi::CString::new("your-256-bit-secret").unwrap();
      let token_ptr = jwt_encode_hs256(claims, secret.as_ptr());
      let token = std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap();
      assert_eq!(
        token.matches('.').count(),
        2,
        "a JWT has exactly 3 segments"
      );
      let token_c = std::ffi::CString::new(token).unwrap();

      let verified = jwt_verify_hs256(token_c.as_ptr(), secret.as_ptr());
      let pairs = read_result_hash(verified).expect("expected Some(claims)");
      assert_eq!(find(&pairs, "name"), Some("John Doe"));
      assert_eq!(find(&pairs, "sub"), Some("1234567890"));
    }
  }

  #[test]
  fn hs256_verify_with_the_wrong_secret_is_nil_not_a_panic() {
    unsafe {
      let claims = alloc_test_hash(&[("sub", "1")]);
      let secret = std::ffi::CString::new("correct-secret").unwrap();
      let wrong = std::ffi::CString::new("wrong-secret").unwrap();
      let token_ptr = jwt_encode_hs256(claims, secret.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();

      let verified = jwt_verify_hs256(token.as_ptr(), wrong.as_ptr());
      assert!(read_result_hash(verified).is_none());
    }
  }

  /// The real jwt.io HS256 debugger example (`claims = {"sub":
  /// "1234567890", "name": "John Doe", "iat": 1516239022}`, `secret =
  /// "your-256-bit-secret"`) with `"name"` hand-tampered from `"John
  /// Doe"` to `"Jane Doe"` in the payload, its original signature left
  /// unchanged — a real, independently-verifiable forged token, not
  /// invented (this plan's own Concrete Proof).
  #[test]
  fn hs256_verify_of_a_hand_tampered_real_world_token_is_nil() {
    unsafe {
      let tampered = std::ffi::CString::new(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkphbmUgRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
      )
      .unwrap();
      let secret = std::ffi::CString::new("your-256-bit-secret").unwrap();
      let verified = jwt_verify_hs256(tampered.as_ptr(), secret.as_ptr());
      assert!(read_result_hash(verified).is_none());
    }
  }

  /// A hand-crafted `"alg":"none"` header with an empty signature
  /// segment — the crate's own `Algorithm` enum (verified directly
  /// against the vendored `algorithms.rs`) has no `None`/unsecured
  /// variant to parse `"none"` into at all, so this fails at header
  /// parsing, never reaching signature verification (this plan's own
  /// Decision log, asserted here as the actual mechanism).
  #[test]
  fn hs256_verify_of_an_alg_none_header_fails_at_header_parsing_not_signature_check() {
    // {"alg":"none","typ":"JWT"} . {"sub":"1234567890"} . (empty sig)
    let forged = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJzdWIiOiIxMjM0NTY3ODkwIn0.";
    let header_result = jsonwebtoken::decode_header(forged);
    assert!(
      header_result.is_err(),
      "the crate's own Algorithm enum has no `none` variant to parse into"
    );
    unsafe {
      let forged_c = std::ffi::CString::new(forged).unwrap();
      let secret = std::ffi::CString::new("any-secret").unwrap();
      let verified = jwt_verify_hs256(forged_c.as_ptr(), secret.as_ptr());
      assert!(read_result_hash(verified).is_none());
    }
  }

  #[test]
  fn hs256_verify_with_issuer_accepts_a_matching_issuer_and_rejects_a_mismatch() {
    unsafe {
      let claims = alloc_test_hash(&[("iss", "my-app"), ("sub", "1")]);
      let secret = std::ffi::CString::new("secret").unwrap();
      let token_ptr = jwt_encode_hs256(claims, secret.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();

      let good_issuer = std::ffi::CString::new("my-app").unwrap();
      let matched =
        jwt_verify_hs256_with_issuer(token.as_ptr(), secret.as_ptr(), good_issuer.as_ptr());
      assert!(read_result_hash(matched).is_some());

      let bad_issuer = std::ffi::CString::new("someone-else").unwrap();
      let mismatched =
        jwt_verify_hs256_with_issuer(token.as_ptr(), secret.as_ptr(), bad_issuer.as_ptr());
      assert!(read_result_hash(mismatched).is_none());
    }
  }

  #[test]
  fn hs256_verify_with_issuer_rejects_a_token_with_no_iss_claim_at_all() {
    unsafe {
      // No `iss` claim in the token at all — `verify_hs256_with_issuer`
      // adds `"iss"` to `required_spec_claims`, so a token that never
      // carried one must fail, not silently pass (see `verify_with_
      // key`'s own doc comment).
      let claims = alloc_test_hash(&[("sub", "1")]);
      let secret = std::ffi::CString::new("secret").unwrap();
      let token_ptr = jwt_encode_hs256(claims, secret.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();
      let issuer = std::ffi::CString::new("my-app").unwrap();
      let verified = jwt_verify_hs256_with_issuer(token.as_ptr(), secret.as_ptr(), issuer.as_ptr());
      assert!(read_result_hash(verified).is_none());
    }
  }

  #[test]
  fn peek_header_reports_the_real_algorithm_with_no_verification() {
    unsafe {
      let claims = alloc_test_hash(&[("sub", "1")]);
      let secret = std::ffi::CString::new("secret").unwrap();
      let token_ptr = jwt_encode_hs256(claims, secret.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();
      let header_ptr = jwt_peek_header(token.as_ptr());
      let header_json = std::ffi::CStr::from_ptr(header_ptr).to_str().unwrap();
      assert!(header_json.contains("\"HS256\""));
    }
  }

  /// RS256 round trip — a fresh, throwaway 1024-bit test-only RSA
  /// keypair, generated in-process (real, disclosed simplification
  /// against production-grade key sizes: this is purely functional
  /// wrapper-marshaling coverage, not a security-strength assertion —
  /// `Rsa.generate_key`, plan 111's own module, is where real key-size
  /// guidance lives).
  #[test]
  fn rs256_round_trip_recovers_the_original_claims_and_rejects_the_wrong_key() {
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::{RsaPrivateKey, RsaPublicKey};

    let mut rng = rand::rngs::OsRng;
    let priv_key = RsaPrivateKey::new(&mut rng, 1024).expect("test RSA keygen");
    let pub_key = RsaPublicKey::from(&priv_key);
    let priv_pem = priv_key
      .to_pkcs8_pem(LineEnding::LF)
      .expect("encode test RSA private key");
    let pub_pem = pub_key
      .to_public_key_pem(LineEnding::LF)
      .expect("encode test RSA public key");

    let other_priv = RsaPrivateKey::new(&mut rng, 1024).expect("second test RSA keygen");
    let other_pub_pem = RsaPublicKey::from(&other_priv)
      .to_public_key_pem(LineEnding::LF)
      .expect("encode second test RSA public key");

    unsafe {
      let claims = alloc_test_hash(&[("sub", "rsa-user")]);
      let priv_pem_c = std::ffi::CString::new(priv_pem.to_string()).unwrap();
      let token_ptr = jwt_encode_rs256(claims, priv_pem_c.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();

      let pub_pem_c = std::ffi::CString::new(pub_pem).unwrap();
      let verified = jwt_verify_rs256(token.as_ptr(), pub_pem_c.as_ptr());
      let pairs = read_result_hash(verified).expect("expected Some(claims)");
      assert_eq!(find(&pairs, "sub"), Some("rsa-user"));

      let other_pub_pem_c = std::ffi::CString::new(other_pub_pem).unwrap();
      let mismatched = jwt_verify_rs256(token.as_ptr(), other_pub_pem_c.as_ptr());
      assert!(read_result_hash(mismatched).is_none());
    }
  }

  /// ES256 round trip — a fresh, throwaway P-256 test-only keypair,
  /// generated in-process via `p256` (a `[dev-dependencies]`-only
  /// addition — see `Cargo.toml`'s own comment — already resolved
  /// transitively at this exact version via `jsonwebtoken`'s own
  /// `rust_crypto` backend, so this test adds zero new entries to the
  /// dependency tree, only visibility).
  #[test]
  fn es256_round_trip_recovers_the_original_claims_and_rejects_the_wrong_key() {
    use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use p256::SecretKey;

    let mut rng = rand::rngs::OsRng;
    let priv_key = SecretKey::random(&mut rng);
    let pub_key = priv_key.public_key();
    let priv_pem = priv_key
      .to_pkcs8_pem(LineEnding::LF)
      .expect("encode test EC private key");
    let pub_pem = pub_key
      .to_public_key_pem(LineEnding::LF)
      .expect("encode test EC public key");

    let other_pub_pem = SecretKey::random(&mut rng)
      .public_key()
      .to_public_key_pem(LineEnding::LF)
      .expect("encode second test EC public key");

    unsafe {
      let claims = alloc_test_hash(&[("sub", "ec-user")]);
      let priv_pem_c = std::ffi::CString::new(priv_pem.to_string()).unwrap();
      let token_ptr = jwt_encode_es256(claims, priv_pem_c.as_ptr());
      let token =
        std::ffi::CString::new(std::ffi::CStr::from_ptr(token_ptr).to_str().unwrap()).unwrap();

      let pub_pem_c = std::ffi::CString::new(pub_pem).unwrap();
      let verified = jwt_verify_es256(token.as_ptr(), pub_pem_c.as_ptr());
      let pairs = read_result_hash(verified).expect("expected Some(claims)");
      assert_eq!(find(&pairs, "sub"), Some("ec-user"));

      let other_pub_pem_c = std::ffi::CString::new(other_pub_pem).unwrap();
      let mismatched = jwt_verify_es256(token.as_ptr(), other_pub_pem_c.as_ptr());
      assert!(read_result_hash(mismatched).is_none());
    }
  }
}
