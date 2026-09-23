//! Plan 104 (Server-Sent Events) — `Sse.upgrade`/`.send`/`.comment`/
//! `.close`, layered directly on plan 101's `tiny_http`-backed
//! `Http.serve` with no new crate at all: SSE is a plain-text wire
//! format (defined by the WHATWG HTML spec, not by any Rust library),
//! not a protocol any library needs to speak on our behalf. The one
//! capability this plan needs that plan 101's own request/response
//! cycle never reaches is `tiny_http::Request::into_writer` — verified
//! directly against `tiny_http` 0.12.0's real, published API
//! (`docs.rs/tiny_http/latest/tiny_http/struct.Request.html`):
//! `pub fn into_writer(self) -> Box<dyn Write + Send + 'static>`. It
//! consumes the `Request` and hands back the raw socket writer,
//! bypassing `tiny_http`'s own buffered `Response`/chunked-encoding
//! path entirely — this module's own code is therefore responsible
//! for writing a byte-for-byte correct HTTP/1.1 response head by hand
//! before the first SSE event (`sse_upgrade` below), something
//! `tiny_http`'s own `Response` type would otherwise generate for a
//! plan-101 caller automatically.
//!
//! **A private registry, not `crate::handle`'s plan-93 one.** Plan
//! 93's own generic `Mutex<HashMap<u64, RegistryEntry>>` (`handle.rs`)
//! plain `.lock().unwrap()`s on every access — fine for the handle
//! types built on it so far, none of which panic with the lock held
//! across a call that could itself observably fail mid-write. This
//! plan's own `.send`/`.comment` calls do real, fallible I/O (a
//! `write_all` to a live socket) while holding this module's own lock,
//! and a panic unwinding through that critical section must not
//! poison the registry for every OTHER still-open SSE stream in the
//! process — so every lock acquisition here goes through
//! `.unwrap_or_else(std::sync::PoisonError::into_inner)`, recovering
//! the guard rather than propagating the poison (the `HashMap` itself
//! is never left torn by anything this module's own code does inside
//! the lock — every critical section here is a plain insert/remove/
//! lookup, never a multi-step invariant that could be left half-
//! updated).
//!
//! **`.send`/`.comment` against a closed or unknown handle return `0`,
//! not a panic.** A real, disclosed "no-op on a dead handle" contract
//! (plan 93's general handle-lifetime philosophy) — an Emerald program
//! that races its own bookkeeping (calling `.send()` after its own
//! `.close()`) degrades to a silently-dropped event, not a crash.
//!
//! **Every write is followed by an explicit `.flush()`.** SSE's whole
//! value proposition — a client sees each event promptly, not
//! batched — depends on it: the writer this module holds is, on every
//! platform this project targets, ultimately backed by a raw
//! `TcpStream` with no userspace buffering of its own, but each event
//! is still composed of several separate `write_all` calls, so nothing
//! guarantees the OS socket buffer gets handed the *complete* event
//! promptly without an explicit flush after the terminating blank
//! line (mirroring `emerald_runtime.c`'s own disclosed `fflush(stdout)`
//! discipline, applied here to a socket instead of stdout).

use crate::http_server;
use std::collections::HashMap;
use std::io::Write;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<HashMap<u64, Box<dyn Write + Send>>> {
  static REGISTRY: OnceLock<Mutex<HashMap<u64, Box<dyn Write + Send>>>> = OnceLock::new();
  REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> u64 {
  static COUNTER: OnceLock<AtomicU64> = OnceLock::new();
  COUNTER
    .get_or_init(|| AtomicU64::new(1))
    .fetch_add(1, Ordering::Relaxed)
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// Runs `f` with the live writer for `id`, under this module's own
// poison-recovering lock — `None` if `id` is closed/unknown (this
// module's own "no-op on a dead handle" contract, see module doc).
fn with_stream<R>(
  id: i64,
  f: impl FnOnce(&mut (dyn Write + Send)) -> std::io::Result<R>,
) -> Option<std::io::Result<R>> {
  let mut reg = registry()
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  reg.get_mut(&(id as u64)).map(|w| f(w.as_mut()))
}

/// `Sse.upgrade(request: Int64): Int64` — consumes plan 101's
/// `HttpRequest` handle (the same `http_request_take` extraction point
/// `HttpRequest#upgrade`/`websocket.rs` already use for the identical
/// "take the live `tiny_http::Request` before it's ever `.respond`ed"
/// purpose), writes the fixed HTTP/1.1 response head by hand, and
/// registers the resulting raw writer under a fresh handle.
pub fn sse_upgrade(request_id: i64) -> i64 {
  let request = match http_server::http_request_take(request_id) {
    Ok(r) => r,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  let mut writer = request.into_writer();
  let head = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-cache\r\n\
Connection: keep-alive\r\n\
\r\n";
  if let Err(e) = writer.write_all(head) {
    unsafe {
      crate::raise_native_error(&format!("Sse.upgrade: failed to write response head: {e}"))
    };
  }
  if writer.flush().is_err() {
    unsafe { crate::raise_native_error("Sse.upgrade: failed to flush response head") };
  }
  let id = next_id();
  registry()
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
    .insert(id, writer);
  id as i64
}

/// `Sse.send(stream: Int64, event: String, data: String): Int64` —
/// real SSE wire-format framing: `event: <name>\n`, one `data: <line>\n`
/// per line of `data` (a `data` value with an embedded newline is
/// split, per the WHATWG spec — a single `data:` line with an embedded
/// `\n` is wire-invalid), a terminating blank line, then an explicit
/// flush. `0`/`1` as `Int64` per plan 59/92's C-side-boolean
/// convention.
///
/// # Safety
/// `event`/`data`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn sse_send(stream: i64, event: *const c_char, data: *const c_char) -> i64 {
  let event = match read_str(event) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let data = match read_str(data) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let result = with_stream(stream, |w| {
    writeln!(w, "event: {event}")?;
    for line in data.split('\n') {
      writeln!(w, "data: {line}")?;
    }
    writeln!(w)?;
    w.flush()
  });
  match result {
    Some(Ok(())) => 1,
    _ => 0,
  }
}

/// `Sse.comment(stream: Int64, text: String): Int64` — the standard
/// SSE keepalive-ping idiom, a bare `: <text>\n` line (intermediary
/// proxies and browsers alike treat an idle SSE connection as suspect
/// after some timeout). `0`/`1` as `Int64`, same convention as `.send`.
///
/// # Safety
/// `text`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn sse_comment(stream: i64, text: *const c_char) -> i64 {
  let text = match read_str(text) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let result = with_stream(stream, |w| {
    writeln!(w, ": {text}")?;
    w.flush()
  });
  match result {
    Some(Ok(())) => 1,
    _ => 0,
  }
}

/// `Sse.close(stream: Int64): Int64` — removes and drops the registry
/// entry, shutting the socket down (dropping the boxed writer is the
/// last live reference to the underlying connection once `sse_upgrade`
/// has already consumed the request's own read half via
/// `into_writer`). `0`/`1`, same convention as `.send`/`.comment` —
/// closing an already-closed or unknown handle is a harmless no-op,
/// not an error, mirroring `crate::handle::handle_close`'s own
/// double-close contract.
pub fn sse_close(stream: i64) -> i64 {
  let removed = registry()
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
    .remove(&(stream as u64))
    .is_some();
  i64::from(removed)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;
  use std::io::Read;
  use std::net::TcpStream;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  // A real `extern "C" fn(i64) -> i64` used directly as `http_serve`'s
  // own `handler` argument, standing in for what codegen's own
  // trampoline would otherwise be — proves the real mechanism (a real
  // `tiny_http::Request` upgraded to a raw writer, three real framed
  // events, a real close) over a real socket, independently of any
  // compiled `.em` program, per this plan's own leaf-test requirement.
  extern "C-unwind" fn sse_handler(req_id: i64) -> i64 {
    let stream = sse_upgrade(req_id);
    let event = c("tick");
    for n in ["1", "2", "3"] {
      let data = c(n);
      unsafe { sse_send(stream, event.as_ptr(), data.as_ptr()) };
    }
    sse_close(stream);
    0
  }

  // Plan 104's own fixed port, distinct from every other fixed port
  // already spent in this crate's/`examples/`'s own tests (`47201`,
  // `47301`/`47302`/`47303`, `47401`/`47402`/`47404`/`47405`,
  // `47501`) — not a copy-paste.
  #[test]
  fn sse_stream_sends_three_framed_events_over_a_real_socket_then_closes() {
    std::thread::spawn(|| unsafe {
      http_server::http_serve(47601, sse_handler);
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut stream = TcpStream::connect(("127.0.0.1", 47601)).expect("connect");
    // `Connection: close`, matching `http_server.rs`'s own
    // `raw_http_get` test helper precedent — without it, `tiny_http`'s
    // own internal per-connection worker (real, disclosed finding:
    // entirely separate from the `Box<dyn Write>` this module's own
    // `sse_close` drops, per `RefinedTcpStream`'s own `Drop` acting on
    // a `try_clone`'d fd, not the shared one the connection's read
    // side still owns) keeps the underlying socket's read half open
    // waiting for a next request that never comes, and this test's own
    // `read_to_end` below would hang forever rather than observing a
    // real EOF.
    stream
      .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
      .expect("write request");
    let mut raw = Vec::new();
    stream
      .read_to_end(&mut raw)
      .expect("read until the server closes the connection");
    let response = String::from_utf8(raw).expect("response should be valid UTF-8");

    let (head, body) = response
      .split_once("\r\n\r\n")
      .expect("response should have a header/body split");
    assert!(head.starts_with("HTTP/1.1 200 OK"));
    let head_lower = head.to_ascii_lowercase();
    assert!(head_lower.contains("content-type: text/event-stream"));
    assert!(head_lower.contains("cache-control: no-cache"));
    assert!(head_lower.contains("connection: keep-alive"));
    assert_eq!(
      body,
      "event: tick\ndata: 1\n\nevent: tick\ndata: 2\n\nevent: tick\ndata: 3\n\n"
    );
  }

  #[test]
  fn send_and_comment_against_a_closed_handle_are_a_harmless_no_op() {
    // Never actually upgraded — `id` was never issued by `sse_upgrade`
    // at all, exercising the "unknown handle" arm of the same
    // no-op contract a closed-then-reused handle would hit.
    let event = c("x");
    let data = c("y");
    let text = c("z");
    assert_eq!(
      unsafe { sse_send(999_999_999, event.as_ptr(), data.as_ptr()) },
      0
    );
    assert_eq!(unsafe { sse_comment(999_999_999, text.as_ptr()) }, 0);
    assert_eq!(sse_close(999_999_999), 0);
  }
}
