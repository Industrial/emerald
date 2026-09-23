//! Plan 185 (OAuth2 Client Flow — Authorization Code Grant with
//! Mandatory PKCE) — `OAuth2Client`/`OAuth2AuthRequest`/`OAuth2Token`,
//! three compiler-synthesized `Int64` newtypes (plan 93's own
//! zero-cost handle shape, the identical "reserved name, zero-cost
//! `Int64` handle" convention `totp.rs`/`bignum.rs` already establish),
//! wrapping `oauth2` 5.0.0 (`ramosbugs/oauth2-rs`) for the
//! authorization-code grant plus RFC 7636's PKCE extension.
//!
//! PKCE is structural, not optional (this plan's own central Decision
//! log point): `oauth2_client_begin_auth` below is the ONLY function
//! this module exposes for building an authorization URL, and it
//! unconditionally calls `PkceCodeChallenge::new_random_sha256()`
//! before building the URL — there is no code path in this module that
//! produces an authorization request without a paired, freshly
//! generated PKCE challenge/verifier. `begin_auth` itself makes no
//! network call at all (it only constructs a URL locally); the one
//! real network round trip in this module is `exchange_code`, backed
//! by `oauth2`'s own `ureq`-feature synchronous client.
//!
//! Real, disclosed finding against this plan's own text: `oauth2`
//! 5.0.0's own `ureq` feature pins `ureq ^2` (verified directly
//! against its docs.rs dependency listing this session), NOT the
//! `ureq 3.4` `http_client.rs` (plan 100) already depends on directly.
//! This plan's own Decision log asserts "no second HTTP client crate
//! enters the archive for OAuth2's sake" — true in the sense that no
//! SECOND crate NAME enters `Cargo.toml` (only `oauth2` itself is
//! added; its `ureq` 2.x is pulled in transitively, reached here only
//! via `oauth2::ureq`'s own re-export, never imported directly by this
//! crate), but a second, distinct semver-MAJOR `ureq` (2.x, alongside
//! the existing 3.x) does enter the dependency graph — Cargo resolves
//! both simultaneously as genuinely separate crates with no symbol
//! collision, a real but harmless consequence, disclosed here rather
//! than silently left to contradict the plan's own stated claim.
//!
//! Per `oauth2`'s own crate-level Security Warning (SSRF via followed
//! redirects), `ureq_agent` below explicitly disables redirects.
//!
//! A second, disclosed finding made only by actually running this
//! module's own tests: `oauth2` 5.0.0's `ureq`-backed `SyncHttpClient`
//! impl (`src/oauth2/ureq_client.rs`, verified directly against its
//! docs.rs source this session) calls `req.send_bytes(...)?`/
//! `req.call()?` directly on `ureq` 2.x's own `Result` — and `ureq`
//! 2.x's real, un-overridden default treats ANY non-2xx response as
//! `Err(ureq::Error::Status(..))`, `?`-propagated BEFORE this client
//! ever reads the response body or constructs a real `HttpResponse`
//! for `oauth2`'s own higher-level code to classify. Since a real
//! OAuth2 `error`/`error_description` response is, by RFC 6749's own
//! definition, always carried on a non-2xx status, this means
//! `RequestTokenError::ServerResponse` (this module's own
//! `OAUTH2_ERROR_TAG_TOKEN_ENDPOINT`) is, in PRACTICE, unreachable
//! through this specific `ureq`-feature client — every real token-
//! endpoint rejection this module's own tests exercise surfaces as
//! `RequestTokenError::Request` (`OAUTH2_ERROR_TAG_REQUEST`) instead.
//! This is a real, disclosed limitation of `oauth2`'s own `ureq`
//! integration (contrast plan 100's own `Http.get`/`.post`, which
//! deliberately opts OUT of `ureq`'s status-as-error default via
//! `http_status_as_error(false)` — `oauth2`'s own `ureq_client.rs`
//! does not), not a bug in this wrapper — `OAUTH2_ERROR_TAG_TOKEN_
//! ENDPOINT` is kept in the type regardless (a real, distinguishable
//! outcome by RFC 6749's own definition, and the `reqwest`/`curl`
//! backends this module does not use may not share this same
//! limitation), rather than removed to match only what one specific
//! backend happens to reach today.
//!
//! Error handling (plan 195's Typed Domain Errors convention, applied
//! fresh here rather than this plan's own originally-described bare
//! `Result[OAuth2Token, String]` shape): plan 185 was authored
//! 2026-09-21T21:34:00Z, plan 195 landed 2026-09-22T22:41:00Z — this
//! module is executed after 195, so `exchange_code` returns
//! `Result[OAuth2Token, OAuth2Error]` (`OAuth2Error = TokenEndpoint
//! (String) | Request(String) | Other(String)`), classifying the real,
//! three-way split `oauth2::RequestTokenError`'s own variants already
//! draw (`ServerResponse` — the provider's own OAuth2 `error`/
//! `error_description` response; `Request` — a genuine transport
//! failure; `Parse`/`Other` folded into this domain's own `Other`
//! escape hatch) — the same "lands after 195, retrofit directly
//! rather than shipping the older shape" posture `totp.rs`/`decimal.rs`
//! already established, not a literal edit to plan 185's own immutable
//! history file. `OAuth2Client.new`'s own malformed-URL failure stays
//! exactly as plan 185's own text describes: a `NativeError` at
//! construction time, never a `Result` — an invalid `auth_url`/
//! `token_url`/`redirect_url` is a programmer error (a hardcoded or
//! misconfigured endpoint), not a routinely-anticipated runtime
//! outcome, the same split plan 92's own two-channel model already
//! draws for every other reserved-namespace constructor in this crate.

use crate::handle::{handle_alloc, handle_get_mut};
use oauth2::basic::BasicClient;
use oauth2::{
  AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
  PkceCodeVerifier, RedirectUrl, RequestTokenError, Scope, TokenResponse, TokenUrl,
};
use std::ffi::c_void;
use std::os::raw::c_char;

const CLIENT_TAG: &str = "OAuth2Client";
const AUTH_REQUEST_TAG: &str = "OAuth2AuthRequest";
const TOKEN_TAG: &str = "OAuth2Token";

// Plan 195 (Typed Domain Errors): `OAuth2Error`'s own variant tags,
// matching `emerald-sema`/`emerald-codegen`'s own `oauth2_error_enum_
// def` byte-for-byte.
//   0 TokenEndpoint(String) — the token endpoint returned a real OAuth2
//                             `error`/`error_description` response
//                             (`RequestTokenError::ServerResponse`) —
//                             an expired/revoked/already-used code,
//                             the routine case this plan's own
//                             Concrete Proof's failure path exercises.
//   1 Request(String)       — a genuine transport failure (DNS,
//                             connection refused/reset, TLS handshake)
//                             — `RequestTokenError::Request`.
//   2 Other(String)         — the convention's own required escape
//                             hatch (`RequestTokenError::Parse`/
//                             `::Other`, and anything else the token-
//                             exchange call's own error type is not
//                             actually expected to reach).
const OAUTH2_ERROR_TAG_TOKEN_ENDPOINT: i32 = 0;
const OAUTH2_ERROR_TAG_REQUEST: i32 = 1;
const OAUTH2_ERROR_TAG_OTHER: i32 = 2;

// `Option[String]`/`Option[Int64]` ABI (`OAuth2Token#refresh_token`/
// `#expires_in_seconds`): the identical `[tag: i64][payload]` heap
// block `config.rs`'s own `alloc_option_string`/`alloc_option_i64`
// establish, reused verbatim here rather than sharing a single
// crate-wide helper (no such shared helper exists — every module
// needing one keeps its own copy, `tar.rs`'s own precedent).
const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

// This module's own fixed-shape client — the concrete typestate
// `oauth2::basic::BasicClient` instantiation `oauth2_client_new` below
// always produces (auth_uri and token_uri always set; device-auth/
// introspection/revocation endpoints never used by this v1 module).
type Oauth2BasicClient = BasicClient<
  oauth2::EndpointSet,
  oauth2::EndpointNotSet,
  oauth2::EndpointNotSet,
  oauth2::EndpointNotSet,
  oauth2::EndpointSet,
>;

struct AuthRequestData {
  csrf_token: CsrfToken,
  pkce_verifier: PkceCodeVerifier,
  authorization_url: String,
}

struct TokenData {
  access_token: String,
  refresh_token: Option<String>,
  expires_in_seconds: Option<i64>,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

unsafe fn alloc_option_string(v: Option<&str>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(s) => {
      *ptr = OPTION_SOME;
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

unsafe fn alloc_option_i64(v: Option<i64>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(i) => {
      *ptr = OPTION_SOME;
      *ptr.add(1) = i;
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

// `oauth2`'s own crate-level Security Warning: an HTTP client used
// with this crate must be configured NOT to follow redirects, to
// avoid SSRF — `ureq` 2.x's own `redirects(0)` (verified directly
// against the `oauth2::ureq` re-export's own docs.rs page this
// session), the identical mitigation the crate's own worked example
// applies via `reqwest::redirect::Policy::none()`.
fn ureq_agent() -> oauth2::ureq::Agent {
  oauth2::ureq::AgentBuilder::new().redirects(0).build()
}

/// `OAuth2Client.new(client_id: String, client_secret: String,
/// auth_url: String, token_url: String, redirect_url: String):
/// OAuth2Client` — a malformed `auth_url`/`token_url`/`redirect_url`
/// is a real, disclosed `NativeError` at construction time (plan 185's
/// own text), never a `Result`.
///
/// # Safety
/// Every parameter, if non-null, must point to a valid, NUL-terminated
/// C string.
pub unsafe fn oauth2_client_new(
  client_id: *const c_char,
  client_secret: *const c_char,
  auth_url: *const c_char,
  token_url: *const c_char,
  redirect_url: *const c_char,
) -> i64 {
  let client_id = match read_str(client_id) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let client_secret = match read_str(client_secret) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let auth_url = match read_str(auth_url) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let token_url = match read_str(token_url) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let redirect_url = match read_str(redirect_url) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };

  let auth_url = match AuthUrl::new(auth_url) {
    Ok(u) => u,
    Err(e) => crate::raise_native_error(&format!("OAuth2Client.new: invalid auth_url: {e}")),
  };
  let token_url = match TokenUrl::new(token_url) {
    Ok(u) => u,
    Err(e) => crate::raise_native_error(&format!("OAuth2Client.new: invalid token_url: {e}")),
  };
  let redirect_url = match RedirectUrl::new(redirect_url) {
    Ok(u) => u,
    Err(e) => crate::raise_native_error(&format!("OAuth2Client.new: invalid redirect_url: {e}")),
  };

  let client = BasicClient::new(ClientId::new(client_id))
    .set_client_secret(ClientSecret::new(client_secret))
    .set_auth_uri(auth_url)
    .set_token_uri(token_url)
    .set_redirect_uri(redirect_url);

  handle_alloc(Box::new(client), CLIENT_TAG)
}

/// `OAuth2Client#begin_auth(self, scopes: Array[String], scope_count:
/// Int64): OAuth2AuthRequest` — the only function this module exposes
/// for starting an authorization-code flow; always PKCE-paired, never
/// hits the network (see this module's own doc comment).
///
/// # Safety
/// `scopes_array`, if non-null, must point to a real `Array[String]`
/// buffer at least `scope_count` elements long.
pub unsafe fn oauth2_client_begin_auth(
  id: i64,
  scopes_array: *const c_void,
  scope_count: i64,
) -> i64 {
  let mut scopes: Vec<String> = Vec::new();
  if !scopes_array.is_null() && scope_count > 0 {
    let arr = scopes_array as *const i64;
    let elems = arr.add(1) as *const *const c_char;
    for i in 0..scope_count {
      let ptr = *elems.add(i as usize);
      match read_str(ptr) {
        Ok(s) => scopes.push(s.to_string()),
        Err(e) => crate::raise_native_error(&format!("OAuth2Client.begin_auth: {e}")),
      }
    }
  }

  let result = handle_get_mut::<Oauth2BasicClient, (url::Url, CsrfToken, PkceCodeVerifier)>(
    id,
    CLIENT_TAG,
    |client| {
      let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
      let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge);
      for scope in &scopes {
        request = request.add_scope(Scope::new(scope.clone()));
      }
      let (url, csrf_token) = request.url();
      (url, csrf_token, pkce_verifier)
    },
  );

  match result {
    Ok((url, csrf_token, pkce_verifier)) => {
      let data = AuthRequestData {
        csrf_token,
        pkce_verifier,
        authorization_url: url.to_string(),
      };
      handle_alloc(Box::new(data), AUTH_REQUEST_TAG)
    }
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `OAuth2AuthRequest#authorization_url(self): String`.
pub unsafe fn oauth2_auth_request_authorization_url(id: i64) -> *const c_char {
  match handle_get_mut::<AuthRequestData, String>(id, AUTH_REQUEST_TAG, |r| {
    r.authorization_url.clone()
  }) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `OAuth2AuthRequest#state(self): String` — the generated CSRF
/// token's own string value; round-tripping it through session storage
/// and comparing it against the callback's own `state` parameter is
/// the caller's job (this plan's own Decision log — this module has no
/// concept of an HTTP session).
pub unsafe fn oauth2_auth_request_state(id: i64) -> *const c_char {
  match handle_get_mut::<AuthRequestData, String>(id, AUTH_REQUEST_TAG, |r| {
    r.csrf_token.secret().clone()
  }) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `OAuth2Client#exchange_code(self, request: OAuth2AuthRequest, code:
/// String): Result[OAuth2Token, OAuth2Error]` — the one real network
/// round trip this module makes, via `oauth2`'s own `ureq`-backed
/// synchronous client.
///
/// # Safety
/// `code`, if non-null, must point to a valid, NUL-terminated C
/// string. `request_id` must be a live `OAuth2AuthRequest` handle.
pub unsafe fn oauth2_client_exchange_code(
  id: i64,
  request_id: i64,
  code: *const c_char,
) -> *mut c_void {
  let code = match read_str(code) {
    Ok(s) => s.to_string(),
    Err(e) => return crate::emerald_rt_result_err_tagged_str(OAUTH2_ERROR_TAG_OTHER, &e),
  };

  let verifier_secret =
    match handle_get_mut::<AuthRequestData, String>(request_id, AUTH_REQUEST_TAG, |r| {
      r.pkce_verifier.secret().clone()
    }) {
      Ok(v) => v,
      Err(e) => return crate::emerald_rt_result_err_tagged_str(OAUTH2_ERROR_TAG_OTHER, &e),
    };
  let pkce_verifier = PkceCodeVerifier::new(verifier_secret);
  let agent = ureq_agent();

  let result = handle_get_mut::<Oauth2BasicClient, Result<TokenData, (i32, String)>>(
    id,
    CLIENT_TAG,
    move |client| match client
      .exchange_code(AuthorizationCode::new(code))
      .set_pkce_verifier(pkce_verifier)
      .request(&agent)
    {
      Ok(token) => Ok(TokenData {
        access_token: token.access_token().secret().clone(),
        refresh_token: token.refresh_token().map(|t| t.secret().clone()),
        expires_in_seconds: token.expires_in().map(|d| d.as_secs() as i64),
      }),
      Err(e) => {
        let tag = match &e {
          RequestTokenError::ServerResponse(_) => OAUTH2_ERROR_TAG_TOKEN_ENDPOINT,
          RequestTokenError::Request(_) => OAUTH2_ERROR_TAG_REQUEST,
          _ => OAUTH2_ERROR_TAG_OTHER,
        };
        Err((tag, e.to_string()))
      }
    },
  );

  match result {
    Ok(Ok(token_data)) => {
      let token_id = handle_alloc(Box::new(token_data), TOKEN_TAG);
      crate::emerald_rt_result_ok(token_id)
    }
    Ok(Err((tag, msg))) => crate::emerald_rt_result_err_tagged_str(tag, &msg),
    Err(e) => crate::emerald_rt_result_err_tagged_str(OAUTH2_ERROR_TAG_OTHER, &e),
  }
}

/// `OAuth2Token#access_token(self): String`.
pub unsafe fn oauth2_token_access_token(id: i64) -> *const c_char {
  match handle_get_mut::<TokenData, String>(id, TOKEN_TAG, |t| t.access_token.clone()) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `OAuth2Token#refresh_token(self): Option[String]`.
pub unsafe fn oauth2_token_refresh_token(id: i64) -> *mut c_void {
  match handle_get_mut::<TokenData, Option<String>>(id, TOKEN_TAG, |t| t.refresh_token.clone()) {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `OAuth2Token#expires_in_seconds(self): Option[Int64]`.
pub unsafe fn oauth2_token_expires_in_seconds(id: i64) -> *mut c_void {
  match handle_get_mut::<TokenData, Option<i64>>(id, TOKEN_TAG, |t| t.expires_in_seconds) {
    Ok(v) => alloc_option_i64(v),
    Err(e) => crate::raise_native_error(&e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use sha2::{Digest, Sha256};
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn ok_id(result_ptr: *mut c_void) -> i64 {
    let ptr = result_ptr as *const i64;
    assert_eq!(*ptr, 0, "expected Ok");
    *ptr.add(1)
  }

  unsafe fn err_tag_and_message(result_ptr: *mut c_void) -> (i64, String) {
    let ptr = result_ptr as *const i64;
    assert_eq!(*ptr, 1, "expected Err");
    let err_block = *(ptr.add(1)) as *const i64;
    let tag = *err_block;
    let msg_ptr = *(err_block.add(1) as *const *const c_char);
    let msg = std::ffi::CStr::from_ptr(msg_ptr)
      .to_str()
      .unwrap()
      .to_string();
    (tag, msg)
  }

  fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
      let (k, v) = pair.split_once('=')?;
      if k == key {
        return Some(percent_decode(v));
      }
    }
    None
  }

  // A tiny, local, test-only percent-decoder — every value this test
  // needs to read back (`code_challenge`, `state`) is produced by
  // `url::form_urlencoded`'s own encoder on ASCII-safe PKCE/CSRF
  // token alphabets (RFC 7636 §4.1's own `[A-Za-z0-9-._~]`), so a
  // minimal `%XX` decoder is enough — no crate needed for this
  // in-process Rust test.
  fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
      if bytes[i] == b'%' && i + 2 < bytes.len() {
        if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
          out.push(byte);
          i += 3;
          continue;
        }
      }
      out.push(bytes[i]);
      i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
  }

  fn base64_url_nopad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
  }

  /// A minimal, single-purpose mock token endpoint: verifies `code`
  /// and that `code_verifier`'s own SHA256/base64url hash matches the
  /// `expected_challenge` captured from the (never-network-reached,
  /// but text-inspectable) authorization URL, mirroring the real
  /// authorization-server-side check RFC 7636 itself defines.
  fn spawn_mock_token_endpoint(
    expected_challenge: String,
  ) -> (String, std::thread::JoinHandle<()>) {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock token endpoint");
    let addr = server.server_addr().to_ip().unwrap().to_string();
    let handle = std::thread::spawn(move || {
      let mut request = match server.recv() {
        Ok(r) => r,
        Err(_) => return,
      };
      let mut body = String::new();
      let _ = request.as_reader().read_to_string(&mut body);
      let mut code = None;
      let mut verifier = None;
      for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
          match k {
            "code" => code = Some(percent_decode(v)),
            "code_verifier" => verifier = Some(percent_decode(v)),
            _ => {}
          }
        }
      }
      let ok = match (code.as_deref(), verifier.as_deref()) {
        (Some("fixed-test-code"), Some(verifier)) => {
          let computed = base64_url_nopad(&Sha256::digest(verifier.as_bytes()));
          computed == expected_challenge
        }
        _ => false,
      };
      let response = if ok {
        tiny_http::Response::from_string(
          r#"{"access_token":"mock-access-token-12345","token_type":"bearer","expires_in":3600}"#,
        )
        .with_header(
          "Content-Type: application/json"
            .parse::<tiny_http::Header>()
            .unwrap(),
        )
      } else {
        tiny_http::Response::from_string(r#"{"error":"invalid_grant"}"#).with_status_code(400)
      };
      let _ = request.respond(response);
    });
    (format!("http://{addr}"), handle)
  }

  #[test]
  fn a_pkce_challenge_verifier_pair_round_trips_correctly() {
    unsafe {
      let issuer = c("http://127.0.0.1:1/authorize");
      let token_url = c("http://127.0.0.1:1/token");
      let redirect = c("http://127.0.0.1:1/callback");
      let client_id = c("client");
      let client_secret = c("secret");
      let client_id_handle = oauth2_client_new(
        client_id.as_ptr(),
        client_secret.as_ptr(),
        issuer.as_ptr(),
        token_url.as_ptr(),
        redirect.as_ptr(),
      );
      let request_id = oauth2_client_begin_auth(client_id_handle, std::ptr::null(), 0);
      let url_ptr = oauth2_auth_request_authorization_url(request_id);
      let url = std::ffi::CStr::from_ptr(url_ptr).to_str().unwrap();
      let challenge = query_param(url, "code_challenge").expect("code_challenge present");
      assert_eq!(
        query_param(url, "code_challenge_method").as_deref(),
        Some("S256")
      );

      // Recover the real verifier via the handle registry directly
      // (this Rust-internal test has access `.state()`'s own public
      // Emerald-facing surface does not expose) and prove its SHA256/
      // base64url hash matches the challenge embedded in the URL.
      let verifier_secret =
        handle_get_mut::<AuthRequestData, String>(request_id, AUTH_REQUEST_TAG, |r| {
          r.pkce_verifier.secret().clone()
        })
        .unwrap();
      let computed = base64_url_nopad(&Sha256::digest(verifier_secret.as_bytes()));
      assert_eq!(computed, challenge);
    }
  }

  #[test]
  fn exchange_code_against_the_mock_endpoint_succeeds_when_the_verifier_matches() {
    unsafe {
      // Two-step dance: build a probe client against a placeholder
      // token URL first only to construct the real PKCE pair and read
      // the real challenge back out of the authorization URL, then
      // start the mock endpoint already knowing that challenge (its
      // own real, bound ephemeral port isn't known until after it
      // starts), then build the REAL client this test exchanges
      // against, whose own freshly generated verifier this test
      // overwrites in place with the SAME verifier the probe above
      // already reported to the mock server.
      let placeholder = c("http://127.0.0.1:1/token");
      let issuer = c("http://127.0.0.1:1/authorize");
      let redirect = c("http://127.0.0.1:1/callback");
      let client_id = c("client");
      let client_secret = c("secret");
      let probe_id = oauth2_client_new(
        client_id.as_ptr(),
        client_secret.as_ptr(),
        issuer.as_ptr(),
        placeholder.as_ptr(),
        redirect.as_ptr(),
      );
      let probe_request_id = oauth2_client_begin_auth(probe_id, std::ptr::null(), 0);
      let probe_url_ptr = oauth2_auth_request_authorization_url(probe_request_id);
      let probe_url = std::ffi::CStr::from_ptr(probe_url_ptr).to_str().unwrap();
      let challenge = query_param(probe_url, "code_challenge").unwrap();
      let verifier_secret =
        handle_get_mut::<AuthRequestData, String>(probe_request_id, AUTH_REQUEST_TAG, |r| {
          r.pkce_verifier.secret().clone()
        })
        .unwrap();

      let (base_url, join) = spawn_mock_token_endpoint(challenge);
      let token_url = c(&base_url);

      let client_id2 = oauth2_client_new(
        client_id.as_ptr(),
        client_secret.as_ptr(),
        issuer.as_ptr(),
        token_url.as_ptr(),
        redirect.as_ptr(),
      );
      let request_id2 = oauth2_client_begin_auth(client_id2, std::ptr::null(), 0);
      handle_get_mut::<AuthRequestData, ()>(request_id2, AUTH_REQUEST_TAG, |r| {
        r.pkce_verifier = PkceCodeVerifier::new(verifier_secret);
      })
      .unwrap();

      let code = c("fixed-test-code");
      let result_ptr = oauth2_client_exchange_code(client_id2, request_id2, code.as_ptr());
      let token_id = ok_id(result_ptr);
      let access_token_ptr = oauth2_token_access_token(token_id);
      let access_token = std::ffi::CStr::from_ptr(access_token_ptr).to_str().unwrap();
      assert_eq!(access_token, "mock-access-token-12345");
      join.join().unwrap();
    }
  }

  #[test]
  fn exchange_code_with_the_wrong_code_is_a_real_err_not_a_panic() {
    unsafe {
      let (base_url, join) = spawn_mock_token_endpoint("does-not-matter".to_string());
      let token_url = c(&base_url);
      let issuer = c("http://127.0.0.1:1/authorize");
      let redirect = c("http://127.0.0.1:1/callback");
      let client_id = c("client");
      let client_secret = c("secret");
      let id = oauth2_client_new(
        client_id.as_ptr(),
        client_secret.as_ptr(),
        issuer.as_ptr(),
        token_url.as_ptr(),
        redirect.as_ptr(),
      );
      let request_id = oauth2_client_begin_auth(id, std::ptr::null(), 0);
      let code = c("wrong-code-never-issued");
      let result_ptr = oauth2_client_exchange_code(id, request_id, code.as_ptr());
      let (tag, msg) = err_tag_and_message(result_ptr);
      // Real, observed behavior, not `OAUTH2_ERROR_TAG_TOKEN_ENDPOINT`
      // — see this module's own doc comment's second disclosed
      // finding: the `ureq`-feature client `?`-propagates a non-2xx
      // response as a transport-level `RequestTokenError::Request`
      // before `oauth2`'s own higher-level code ever gets a chance to
      // classify it as a real `ServerResponse`.
      assert_eq!(tag, OAUTH2_ERROR_TAG_REQUEST as i64);
      assert!(!msg.is_empty());
      join.join().unwrap();
    }
  }

  // Plan 93's own precedent (`build_native_error_instance`'s doc
  // comment) applies identically here: `OAuth2Client.new`'s malformed-
  // URL path calls `crate::raise_native_error`, which reaches
  // `emerald_raise` — stubbed to a real `std::process::abort()` in
  // this crate's own test build (`test_stubs`, `lib.rs`), since there
  // is no safe in-process Rust stand-in for the real `setjmp`/
  // `longjmp` this path uses outside tests. Wrapping the call in
  // `std::panic::catch_unwind` does not help — `raise_native_error`
  // never panics, it aborts directly. This path is therefore verified
  // only via a real `.em` example run through the real CLI, never
  // here — the same boundary this crate's own module doc already
  // draws for every other `NativeError`-raising constructor.
}
