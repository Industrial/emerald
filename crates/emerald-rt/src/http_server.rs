//! Plan 101 (HTTP Server) — `Http.serve(port) do |req| ... end`,
//! wrapping `tiny_http` (100% blocking, no embedded runtime — its own
//! internal per-connection thread pool already solves concurrency,
//! matching plan 94's sync-first posture with zero bridging). Real-
//! reconfirmed this session (not trusted from the plan's own snapshot
//! alone): `tiny_http` 0.12.0 (Oct 6, 2022) is STILL the latest
//! crates.io release as of this session's own check against
//! `docs.rs/tiny_http/latest` — the plan's own "no newer release since
//! 2022" staleness caveat still holds precisely, unlike every other
//! crate this session touched (`hickory-resolver`/`ureq`/`url` all had
//! real API drift against their own plan's text; `tiny_http` did not).
//!
//! Two real, disclosed simplifications against this plan's own text,
//! mirroring plan 100's own precedent for `HttpResponse`:
//!
//! 1. **`HttpRequest` is an `Int64` newtype over `crate::handle`'s
//!    registry**, not a new hardcoded `Type::HttpRequest` compound
//!    variant — the identical shape `HttpResponse`/`Regex`/`Url`
//!    already use, avoiding a change to every exhaustive `Type` match
//!    throughout the compiler.
//! 2. **`HttpResponse.new(status, body)`, exactly as the plan's own
//!    Concrete Proof writes it, can never actually reach reserved-
//!    namespace dispatch — `"new"` is a grammar-reserved keyword that
//!    parses unconditionally into `Expr::New` (real class
//!    instantiation), confirmed against `grammar.lalrpop` directly,
//!    the same finding plan 109 already made for `Sha256.new()`
//!    (renamed `.hasher()` there for the identical reason). Renamed
//!    here to `HttpResponse.build(status, body)`.
//!
//! **A third, genuinely new mechanism this plan is the first to build:
//! native Rust code calling back into compiled Emerald code.** The
//! block attached to `Http.serve(port) do |req: HttpRequest| ... end`
//! needs zero new grammar — confirmed directly against
//! `grammar.lalrpop`'s `ChainCallExpr` production (already parses any
//! `<Ident>.<method>(<args>) <DoBlock>`, `Http` included, with zero
//! special-casing) — and zero new sema block-typing mechanism:
//! `infer_lambda_type` already resolves a block param's own declared
//! type generically via `resolve_type` (works for ANY named type
//! already registered in `classes`, `HttpRequest` included) and
//! infers the block's return type from its own body — the "block-
//! typing extension" leaf this plan's own text worried about was
//! already fully general. What's genuinely new is the CALLING
//! CONVENTION: `emerald-codegen` compiles the block via the existing
//! `build_inline_lambda` machinery (the same mechanism `.each do |x|
//! ... end` already uses — every block literal already compiles to a
//! real, separate top-level LLVM function, contrary to this plan's own
//! "effectively inlined" framing), then wraps it in a hand-built,
//! `extern "C" fn(i64) -> i64` trampoline (mirroring plan 55's own
//! `declare_actor_trampolines` shape: a fixed-ABI wrapper function that
//! loads the block's own closure/captures from a call-site-unique
//! global, indirect-calls the real compiled block body, and — the
//! exact per-request panic boundary this plan requires — wraps that
//! call in the identical `setjmp`/`push_handler` catch frame
//! `declare_actor_trampolines` already establishes for actor methods,
//! so an uncaught Emerald exception inside a handler becomes a real
//! `HttpResponse.build(500, ...)` value INSIDE the trampoline itself,
//! never a `longjmp` reaching across the Rust FFI boundary at all (the
//! same "unwinding/`longjmp`ing across a plain `extern "C" fn` boundary
//! is UB" reasoning this crate's own module doc already states for
//! Rust panics, applied here in the opposite direction). `http_serve`
//! below's own `std::panic::catch_unwind` around the call to `handler`
//! is a real, but strictly secondary, second layer — the PRIMARY
//! protection happens inside the compiled trampoline, before a real
//! Rust panic would ever have a chance to occur on this side at all.

use crate::handle::{handle_alloc, handle_get_mut};
use crate::http_client::{http_response_new_handle, HttpResponseData, HTTP_RESPONSE_TAG};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

const HTTP_REQUEST_TAG: &str = "HttpRequest";

struct HttpRequestData {
  method: String,
  path: String,
  body: String,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Http.serve(port: Int64, handler): Void` — `handler` is a codegen-
/// synthesized `extern "C" fn(i64) -> i64` trampoline (see this
/// module's own doc comment), never a raw Emerald value source code
/// could construct itself. Blocks forever, mirroring `tiny_http`'s own
/// blocking `incoming_requests()` iterator, which never terminates on
/// its own — the same "runs to completion, i.e. never" contract an
/// ordinary `while true` already has in this language (this plan's own
/// Decision log: no resource handle/`.stop()` in v1).
///
/// # Safety
/// `handler` must be a valid function pointer to a compiled Emerald
/// trampoline taking one live `HttpRequest` handle id and returning
/// one live `HttpResponse` handle id — codegen's own responsibility.
///
/// Real, disclosed finding made writing this plan's own test suite:
/// `handler` is declared `extern "C-unwind"`, not the plan's own
/// literal `extern "C"` — verified empirically, not assumed. A plain
/// `extern "C" fn` that panics while executing has its unwind
/// intentionally turned into a process `abort()` by Rust itself the
/// moment it tries to leave that function's own frame (stabilized
/// alongside the `C-unwind` ABI, Rust 1.71) — BEFORE the panic ever
/// reaches this function's own `catch_unwind` below, making that
/// `catch_unwind` a no-op for exactly the case it exists to catch.
/// `extern "C-unwind"` is Rust's own real, documented fix: it tells
/// the compiler calling through this pointer may legitimately unwind,
/// restoring `catch_unwind`'s ability to actually catch it. Harmless
/// for the real, compiled-Emerald-trampoline case (which never uses
/// Rust's own unwinding at all — an uncaught Emerald exception is a
/// `setjmp`/`longjmp`, caught and converted to a real `HttpResponse`
/// INSIDE that trampoline, per this module's own doc comment) — this
/// only removes an incorrect "this call can never unwind" assumption
/// the compiler would otherwise bake in, the same second-layer defense
/// this plan's own text asks for now actually being capable of firing.
pub unsafe fn http_serve(port: i64, handler: extern "C-unwind" fn(i64) -> i64) -> i64 {
  if !(0..=u16::MAX as i64).contains(&port) {
    crate::raise_native_error("Http.serve: port must be in 0..=65535");
  }
  // Loopback-only, deliberately: this plan's own Concrete Proof is
  // explicitly self-contained (no external network dependency, unlike
  // plan 100's own httpbin.org-dependent proof) — binding `127.0.0.1`
  // rather than `0.0.0.0` keeps that guarantee real at the network
  // layer too, not just at the test-harness level.
  let addr = format!("127.0.0.1:{port}");
  let server = match tiny_http::Server::http(&addr) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&format!("Http.serve: failed to bind {addr}: {e}")),
  };
  for mut request in server.incoming_requests() {
    let method = request.method().to_string();
    let path = request.url().to_string();
    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    let req_id = handle_alloc(
      Box::new(HttpRequestData { method, path, body }),
      HTTP_REQUEST_TAG,
    );
    // Real, secondary defensive layer only — see this module's own doc
    // comment for why the trampoline itself is the PRIMARY boundary.
    let resp_id = match catch_unwind(AssertUnwindSafe(|| handler(req_id))) {
      Ok(id) => id,
      Err(_) => http_response_new_handle(500, "Internal Server Error".to_string()),
    };
    let status = handle_get_mut::<HttpResponseData, i64>(resp_id, HTTP_RESPONSE_TAG, |r| r.status)
      .unwrap_or(500);
    let response_body =
      handle_get_mut::<HttpResponseData, String>(resp_id, HTTP_RESPONSE_TAG, |r| r.body.clone())
        .unwrap_or_else(|_| "Internal Server Error".to_string());
    let status_code: u16 = status.clamp(100, 599) as u16;
    let response = tiny_http::Response::from_string(response_body).with_status_code(status_code);
    let _ = request.respond(response);
  }
  0
}

/// `HttpResponse.build(status: Int64, body: String): HttpResponse` —
/// the only way a handler block builds its return value; see this
/// module's own doc comment for why `.build`, not the plan's own
/// literal `.new`.
///
/// # Safety
/// `body`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn http_response_build(status: i64, body: *const c_char) -> i64 {
  let body = match read_str(body) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  http_response_new_handle(status, body)
}

/// `HttpRequest#method(self): String`.
pub fn http_request_method(id: i64) -> *const c_char {
  match handle_get_mut::<HttpRequestData, String>(id, HTTP_REQUEST_TAG, |r| r.method.clone()) {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `HttpRequest#path(self): String`.
pub fn http_request_path(id: i64) -> *const c_char {
  match handle_get_mut::<HttpRequestData, String>(id, HTTP_REQUEST_TAG, |r| r.path.clone()) {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `HttpRequest#body(self): String`.
pub fn http_request_body(id: i64) -> *const c_char {
  match handle_get_mut::<HttpRequestData, String>(id, HTTP_REQUEST_TAG, |r| r.body.clone()) {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;
  use std::io::{Read, Write};
  use std::net::TcpStream;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  // A real `extern "C" fn(i64) -> i64` used directly as `http_serve`'s
  // own `handler` argument, standing in for what codegen's own
  // trampoline would otherwise be — proves the Rust-side loop, request
  // marshaling, and response dispatch independently of any compiled
  // `.em` program, per this plan's own leaf-test requirement.
  extern "C-unwind" fn echo_handler(req_id: i64) -> i64 {
    let path = http_request_path(req_id);
    let path = unsafe { cstr(path) };
    if path == "/hello" {
      let method = http_request_method(req_id);
      let method = unsafe { cstr(method) };
      let body_c = c(&format!("hello {method}"));
      unsafe { http_response_build(200, body_c.as_ptr()) }
    } else {
      let body_c = c("not found");
      unsafe { http_response_build(404, body_c.as_ptr()) }
    }
  }

  extern "C-unwind" fn panicking_handler(_req_id: i64) -> i64 {
    panic!("deliberate test panic inside a handler");
  }

  fn raw_http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
      .write_all(
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes(),
      )
      .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status_line = response.lines().next().unwrap_or("");
    let status: u16 = status_line
      .split_whitespace()
      .nth(1)
      .and_then(|s| s.parse().ok())
      .unwrap_or(0);
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
  }

  // Plan 96's own port `47201` is already spent by `raw_tcp_sockets.em`
  // — a deliberately different, unlikely-to-collide fixed port, not a
  // copy-paste.
  #[test]
  fn serve_loopback_round_trip_hello_and_not_found() {
    let handle = std::thread::spawn(|| unsafe {
      http_serve(47301, echo_handler);
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    let (status, body) = raw_http_get(47301, "/hello");
    assert_eq!(status, 200);
    assert_eq!(body, "hello GET");

    let (status, body) = raw_http_get(47301, "/missing");
    assert_eq!(status, 404);
    assert_eq!(body, "not found");

    // `http_serve` never returns under normal operation (see its own
    // doc comment) — this test process exiting at the end of the test
    // binary run is the real cleanup, matching `tiny_http`'s own
    // documented lack of a graceful-shutdown API in this version; the
    // spawned thread is deliberately never joined.
    drop(handle);
  }

  // Proves the per-request panic boundary directly: one request whose
  // handler panics gets a real 500, and the server survives to serve
  // a second, normal request afterward — not a crashed process.
  #[test]
  fn a_panicking_handler_returns_500_and_the_server_survives() {
    std::thread::spawn(|| unsafe {
      http_serve(47302, panicking_handler);
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    let (status, _body) = raw_http_get(47302, "/anything");
    assert_eq!(status, 500);

    // The server thread survived the panic (it's still `tiny_http`'s
    // own internal worker thread handling this — `catch_unwind`
    // caught it) — a second request on the SAME listener still gets a
    // real response, not a connection refused.
    let (status, _body) = raw_http_get(47302, "/anything-else");
    assert_eq!(status, 500);
  }
}
