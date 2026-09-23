//! Plan 106 (Advanced Async HTTP, hyper-direct) — `Http2Client.get`/
//! `.last_status`, wrapping `hyper` + `hyper-util`'s connection-
//! pooling `hyper_util::client::legacy::Client` directly, layered on
//! top of plan 94's shared `crate::tokio_rt()` — the heavier, opt-in
//! upgrade path for Emerald code that issues many requests to the
//! same host and wants persistent connection reuse, not a replacement
//! for plan 100's simpler, per-call `ureq`-backed `Http.get`/`.post`
//! (`http_client.rs`), which stays the default.
//!
//! **The shared pooled `Client` and the async runtime are both
//! singletons, constructed once per process** — the entire mechanism
//! this plan exists to prove (see this plan's own Decision log,
//! `history/2026-09-21T201500Z-plan-106-advanced-async-http.md`): a
//! fresh `Client` per call would defeat connection reuse while still
//! compiling and returning correct-looking output, a silent
//! regression this module deliberately avoids by construction (one
//! `OnceLock<Client<...>>`, never one per call — reusing plan 97's own
//! shared `crate::tokio_rt()` rather than starting a second runtime).
//!
//! **Real, disclosed scope decision: HTTP only in this v1, not
//! HTTP+HTTPS.** The plan's own leaf text names an "rustls-backed
//! connector when the target is `https://`, reusing plan 99's TLS
//! setup" — found, while actually implementing this, to require more
//! than plan 99's own `rustls` setup can provide as-is: `tls.rs`'s own
//! `rustls::StreamOwned` is a genuinely *synchronous* `Read`/`Write`
//! adapter built directly on `std::net::TcpStream` (that's its whole
//! stated advantage for plan 99's own sync-first use), while
//! `hyper`/`hyper-util`'s connector trait (`tower::Service<Uri>`) is
//! asynchronous end to end — bridging the two for real needs an async
//! TLS layer (`tokio-rustls`, or the `hyper-rustls` crate built on top
//! of it), neither of which is in this plan's own declared dependency
//! leaf (`hyper`/`hyper-util`/`http-body-util` only). Rather than
//! quietly add an unplanned dependency or ship a fake/broken HTTPS
//! path, `Http2Client.get` returns a clear, disclosed error for
//! `https://` URLs — a real, honest gap, not a silent omission,
//! matching the plan's own already-established pattern of naming
//! structural gaps in its Decision log rather than papering over
//! them. `http://` is fully supported, including the connection-reuse
//! pooling that is this plan's actual stated purpose, and is all the
//! Concrete Proof/`leaf-connection-reuse-proof` below need —
//! `tiny_http` (this plan's own real local proof target, plan 101)
//! has no TLS support of its own to test against anyway.
//!
//! **Boolean/handle conventions reused verbatim from plan 59/92/93,
//! per this plan's own Decision log**: `Http2Client.get`'s error path
//! (a connection/parse failure, or an unsupported `https://` scheme)
//! sets `Http2Client.last_status()` to `-1` and returns an empty
//! `String`, rather than raising — a plain `Int64`-status-check
//! convention, matching plan 100's own `Http.get`/`.post` shape one
//! layer up (no new marshaling convention introduced).

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;

// Sentinel per this module's own doc comment / plan 106's Decision
// log — no real HTTP status code is negative.
const ERROR_STATUS: i64 = -1;

static LAST_STATUS: AtomicI64 = AtomicI64::new(ERROR_STATUS);

// One process-wide, lazily-initialized pooled client — see this
// module's own doc comment for why this MUST stay a single shared
// instance, never one per call.
fn client() -> &'static Client<HttpConnector, Empty<Bytes>> {
  static CLIENT: OnceLock<Client<HttpConnector, Empty<Bytes>>> = OnceLock::new();
  CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

async fn do_get(url: &str) -> Result<(i64, String), String> {
  let uri: Uri = url
    .parse()
    .map_err(|e: hyper::http::uri::InvalidUri| e.to_string())?;
  if uri.scheme_str() == Some("https") {
    return Err(
      "Http2Client.get: https:// is not supported in this build (HTTP-only v1 \
       — see http2_client.rs's own module doc for the real, disclosed reason)"
        .to_string(),
    );
  }
  let req = Request::builder()
    .method(Method::GET)
    .uri(uri)
    .body(Empty::<Bytes>::new())
    .map_err(|e| e.to_string())?;
  let resp = client().request(req).await.map_err(|e| e.to_string())?;
  let status = resp.status().as_u16() as i64;
  let collected = resp
    .into_body()
    .collect()
    .await
    .map_err(|e| e.to_string())?;
  let body_bytes = collected.to_bytes();
  let body = String::from_utf8_lossy(&body_bytes).into_owned();
  Ok((status, body))
}

/// `Http2Client.get(url: String): String` — buffers the whole response
/// body (per this plan's own Decision log — true chunk-streamed
/// bodies are structurally deferred, see the plan doc). Never raises:
/// a transport/parse/unsupported-scheme failure sets
/// `Http2Client.last_status()` to `-1` and returns an empty `String`.
///
/// # Safety
/// `url`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn http2_client_get(url: *const c_char) -> *const c_char {
  let url = match read_str(url) {
    Ok(s) => s,
    Err(_) => {
      LAST_STATUS.store(ERROR_STATUS, Ordering::SeqCst);
      return crate::alloc_and_copy_str("");
    }
  };
  match crate::tokio_rt().block_on(do_get(url)) {
    Ok((status, body)) => {
      LAST_STATUS.store(status, Ordering::SeqCst);
      crate::alloc_and_copy_str(&body)
    }
    Err(_) => {
      LAST_STATUS.store(ERROR_STATUS, Ordering::SeqCst);
      crate::alloc_and_copy_str("")
    }
  }
}

/// `Http2Client.last_status(): Int64` — the HTTP status of the most
/// recent `Http2Client.get` call on this process, or `-1` if it
/// failed before a real status was received.
pub fn http2_client_last_status() -> i64 {
  LAST_STATUS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;
  use std::io::{Read, Write};
  use std::net::TcpListener;
  use std::sync::atomic::AtomicUsize;
  use std::sync::Arc;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  // `LAST_STATUS` is one process-wide global (per this module's own
  // doc comment / plan 106's Decision log) — real, but harmless for
  // production use (Emerald programs are single-threaded from this
  // module's own point of view). It does mean two of THIS module's
  // own tests calling `http2_client_get`/`.last_status` concurrently
  // could race each other under `cargo test`'s default parallel
  // execution; this lock keeps this module's own two tests serialized
  // against each other (not a production fix — there's nothing to fix
  // in production code for a per-process design the plan itself
  // specifies) so neither observes the other's in-flight status.
  static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

  // `leaf-connection-reuse-proof`: a minimal, hand-rolled HTTP/1.1
  // server — deliberately NOT `tiny_http` (already a sibling
  // dependency, plan 101), since its own API gives no visibility into
  // real `TcpListener::accept` counts, the one thing this test exists
  // to check (see the plan's own Decision log: output text alone
  // cannot distinguish a pooled connection from three fresh ones).
  // Keeps every accepted connection alive across repeated requests
  // (`Connection: keep-alive`, hyper's own HTTP/1.1 default) so a
  // correctly pooling client has a real reason to reuse it.
  #[test]
  fn three_sequential_gets_to_the_same_host_reuse_one_real_tcp_connection() {
    let _guard = TEST_LOCK.lock().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept_count = Arc::new(AtomicUsize::new(0));
    let accept_count_srv = accept_count.clone();
    std::thread::spawn(move || {
      for stream in listener.incoming() {
        let mut stream = match stream {
          Ok(s) => s,
          Err(_) => break,
        };
        accept_count_srv.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
          let mut buf = [0u8; 4096];
          loop {
            let mut req = Vec::new();
            loop {
              let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
              };
              req.extend_from_slice(&buf[..n]);
              if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
              }
            }
            let body = b"pong";
            let resp = format!(
              "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
              body.len()
            );
            if stream.write_all(resp.as_bytes()).is_err() {
              return;
            }
            if stream.write_all(body).is_err() {
              return;
            }
          }
        });
      }
    });

    let url = format!("http://127.0.0.1:{port}/");
    let url_c = c(&url);
    for _ in 0..3 {
      unsafe {
        let body = cstr(http2_client_get(url_c.as_ptr()));
        assert_eq!(body, "pong");
        assert_eq!(http2_client_last_status(), 200);
      }
    }
    assert_eq!(
      accept_count.load(Ordering::SeqCst),
      1,
      "expected the shared pooled client to reuse one real TCP connection across three sequential requests"
    );
  }

  // Proves the error path (a genuine transport failure) sets the
  // disclosed `-1` sentinel and an empty body, never raises — the
  // identical convention plan 100's own `Http.get` established one
  // layer up, per this plan's own Decision log.
  #[test]
  fn get_against_an_unresolvable_host_returns_empty_string_and_sets_last_status_to_negative_one() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
      let url = c("http://this-host-does-not-exist.invalid/");
      let body = cstr(http2_client_get(url.as_ptr()));
      assert_eq!(body, "");
      assert_eq!(http2_client_last_status(), -1);
    }
  }

  // Proves the disclosed `https://` scope gap (this module's own doc
  // comment) surfaces through the identical sentinel convention, not
  // a panic or a silently-wrong plaintext connection attempt.
  #[test]
  fn get_against_an_https_url_returns_the_disclosed_unsupported_sentinel() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
      let url = c("https://example.invalid/");
      let body = cstr(http2_client_get(url.as_ptr()));
      assert_eq!(body, "");
      assert_eq!(http2_client_last_status(), -1);
    }
  }
}
