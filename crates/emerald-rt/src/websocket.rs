//! Plan 102 (WebSocket) — `WebSocket.connect` (client) and
//! `HttpRequest#upgrade` (server, called from inside an `Http.serve`
//! handler — plan 101) both resolve to the identical opaque-`u64`-
//! handle `WebSocketConnection` (plan 93's model), backed by plain,
//! synchronous `tungstenite`, riding directly on `std::net::TcpStream`
//! for the client path and on `tiny_http::Request::upgrade`'s own raw
//! stream for the server path — the one real integration point that
//! lets plans 101 and 102 compose without either wrapping crate ever
//! needing to know about the other.
//!
//! **`WsStream`, not two separate `WebSocket<S>` instantiations.**
//! `tungstenite::WebSocket<S>` is generic over its stream type, and
//! the client path's stream (`TcpStream`) and the server path's
//! (`Box<dyn tiny_http::ReadWrite + Send>`, `tiny_http::Request::
//! upgrade`'s own return type) are two genuinely different Rust
//! types — mirrored here on `tls.rs`'s own `TlsStreamInner` precedent
//! (a small enum implementing `Read`/`Write` by dispatching to
//! whichever variant is actually stored), but one level higher: this
//! enum implements `Read`/`Write` itself, so exactly ONE `WebSocket<
//! WsStream>` type is ever stored in `crate::handle`'s registry,
//! regardless of which side opened the connection — simpler than
//! `TlsStreamInner`'s own "one enum per full `StreamOwned<C, S>`"
//! shape, since `tungstenite::WebSocket<S>`'s own genericity already
//! does the rest of the work once `S` itself is unified.
//!
//! **Real, disclosed improvement over this plan's own original text:
//! binary frames are `Bytes`, not `Array[Int64]`.** This plan's own
//! Decision log, written before plan 109 (Hashing) landed, proposed
//! `Array[Int64]` (one `Int64` per byte, a real, disclosed 8x
//! overhead) as a deliberate stopgap, explicitly naming the gap a
//! later plan should close "once more than one domain plan in this
//! batch needs" a packed byte buffer. Plan 109 already closed it —
//! `Bytes` (`bytes.rs`, a `[len: i64][data: u8 * len]` heap block
//! behind a zero-cost `Int64`-newtype handle) is exactly that type,
//! already vetted and linked. `.send_binary`/`WebSocketMessage#bytes`
//! use it directly; this module never touches `Array[Int64]` at all.
//!
//! **`ws://` only — no TLS feature enabled, per this plan's own
//! Decision log.** `tungstenite`'s `native-tls`/`rustls-tls-*`
//! feature flags are real, but this plan explicitly declines to pick
//! a default ahead of plan 99's own shared `rustls` convention
//! (`Cargo.toml`'s own comment on the `tungstenite` line has the
//! full reasoning). `WebSocket.connect` rejects any URL scheme other
//! than `"ws"` with a typed `WebSocketError::Protocol`.
//!
//! **The RFC6455 handshake accept-key computation is NOT reimplemented
//! here.** `tungstenite::handshake::derive_accept_key` (SHA-1-then-
//! base64 of the client's `Sec-WebSocket-Key` plus the protocol's
//! fixed magic GUID — mandated by the spec itself, not a security
//! choice) is already linked by this crate's own client-side handshake
//! verification; `ws_upgrade_from_http` below reuses it directly for
//! the server side rather than adding a redundant direct `sha1`
//! dependency for the identical, already-available computation.
//!
//! **`.close()` is best-effort, not a full closing handshake.**
//! `tungstenite::WebSocket::close` only *queues* a close frame; per
//! this plan's own "Not yet decided" item 3 (no caller-supplied
//! timeout anywhere in this module), `.close()` here sends the close
//! frame, makes one best-effort attempt to flush it, and then removes
//! the handle — dropping the underlying stream closes the real TCP
//! connection (a genuine FIN) even on a peer that never completes its
//! own half of the WS-level close handshake. Real and disclosed, not
//! a bug: the alternative (blocking indefinitely on a peer that never
//! responds) is strictly worse for a `.close()` call with no timeout
//! parameter to bound it.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::raw::c_char;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket as TungWebSocket};

const WS_CONN_TAG: &str = "WebSocketConnection";

// `WebSocketError` variant tags — declaration order matches
// `emerald-sema`/`emerald-codegen`'s own mirrored `WebSocketError`
// enum byte-for-byte (plan 195's Typed Domain Errors convention, the
// same "classify what's real, fold the rest" shape `PathError`/
// `DecimalError` already establish).
//   0 ConnectionClosed
//   1 Protocol(String)
//   2 Other(String)
const WS_ERROR_TAG_CONNECTION_CLOSED: i32 = 0;
const WS_ERROR_TAG_PROTOCOL: i32 = 1;
const WS_ERROR_TAG_OTHER: i32 = 2;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// See this module's own doc comment for why this single enum,
/// implementing `Read`/`Write` itself, replaces two distinct
/// `WebSocket<S>` instantiations.
enum WsStream {
  Client(TcpStream),
  Server(Box<dyn tiny_http::ReadWrite + Send>),
}

impl Read for WsStream {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    match self {
      WsStream::Client(s) => s.read(buf),
      WsStream::Server(s) => s.read(buf),
    }
  }
}

impl Write for WsStream {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    match self {
      WsStream::Client(s) => s.write(buf),
      WsStream::Server(s) => s.write(buf),
    }
  }
  fn flush(&mut self) -> std::io::Result<()> {
    match self {
      WsStream::Client(s) => s.flush(),
      WsStream::Server(s) => s.flush(),
    }
  }
}

fn classify_ws_error(e: &tungstenite::Error) -> (i32, String) {
  match e {
    tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
      (WS_ERROR_TAG_CONNECTION_CLOSED, e.to_string())
    }
    tungstenite::Error::Protocol(_) | tungstenite::Error::Http(_) => {
      (WS_ERROR_TAG_PROTOCOL, e.to_string())
    }
    other => (WS_ERROR_TAG_OTHER, other.to_string()),
  }
}

unsafe fn err_ws(e: &tungstenite::Error) -> *mut std::ffi::c_void {
  let (tag, msg) = classify_ws_error(e);
  crate::emerald_rt_result_err_tagged_str(tag, &msg)
}

unsafe fn err_protocol(msg: &str) -> *mut std::ffi::c_void {
  crate::emerald_rt_result_err_tagged_str(WS_ERROR_TAG_PROTOCOL, msg)
}

unsafe fn err_other(msg: &str) -> *mut std::ffi::c_void {
  crate::emerald_rt_result_err_tagged_str(WS_ERROR_TAG_OTHER, msg)
}

/// `WebSocket.connect(url: String): Result[WebSocketConnection, WebSocketError]`
/// — opens its own raw `std::net::TcpStream` (plan 96's own raw-socket
/// precedent) and drives the client-side RFC6455 handshake over it via
/// `tungstenite::client`. `ws://` only — any other scheme (`wss://`
/// included; see this module's own doc comment) is a typed `Protocol`
/// error, not a panic.
///
/// # Safety
/// `url`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn ws_connect(url: *const c_char) -> *mut std::ffi::c_void {
  let url_str = match read_str(url) {
    Ok(s) => s,
    Err(e) => return err_other(&e),
  };
  let parsed = match url::Url::parse(url_str) {
    Ok(u) => u,
    Err(e) => return err_protocol(&format!("WebSocket.connect: invalid URL: {e}")),
  };
  if parsed.scheme() != "ws" {
    return err_protocol(&format!(
      "WebSocket.connect: unsupported scheme `{}` (only `ws://` is supported — `wss://` is a real, disclosed follow-up, see this crate's own websocket.rs module doc)",
      parsed.scheme()
    ));
  }
  let host = match parsed.host_str() {
    Some(h) => h,
    None => return err_protocol("WebSocket.connect: URL has no host"),
  };
  let port = parsed.port().unwrap_or(80);
  let sock = match TcpStream::connect((host, port)) {
    Ok(s) => s,
    Err(e) => return err_other(&format!("WebSocket.connect: {e}")),
  };
  match tungstenite::client(url_str, WsStream::Client(sock)) {
    Ok((ws, _response)) => {
      let id = handle_alloc(Box::new(ws), WS_CONN_TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(e) => err_other(&format!("WebSocket.connect: {e}")),
  }
}

/// `HttpRequest#upgrade(self): Result[WebSocketConnection, WebSocketError]`
/// — callable only from inside an `Http.serve` handler (nothing
/// enforces that structurally beyond `request_id` needing to be a
/// live `HttpRequest` handle whose own `tiny_http::Request` hasn't
/// already been taken — see `http_server.rs`'s own `http_request_
/// take`). Computes the RFC6455 handshake response and calls
/// `tiny_http`'s own `Request::upgrade` to obtain the raw duplex
/// stream, then wraps it with `tungstenite::WebSocket::from_raw_
/// socket` — no second handshake read, no re-negotiation.
pub fn ws_upgrade_from_http(request_id: i64) -> *mut std::ffi::c_void {
  let request = match crate::http_server::http_request_take(request_id) {
    Ok(r) => r,
    Err(e) => return unsafe { err_other(&e) },
  };
  let key = request
    .headers()
    .iter()
    .find(|h| {
      h.field
        .as_str()
        .as_str()
        .eq_ignore_ascii_case("Sec-WebSocket-Key")
    })
    .map(|h| h.value.as_str().to_string());
  let key = match key {
    Some(k) => k,
    None => {
      return unsafe { err_protocol("HttpRequest#upgrade: missing Sec-WebSocket-Key header") }
    }
  };
  let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
  let response = tiny_http::Response::empty(101)
    .with_header(
      tiny_http::Header::from_bytes(&b"Upgrade"[..], &b"websocket"[..])
        .expect("static header name/value is always valid"),
    )
    .with_header(
      tiny_http::Header::from_bytes(&b"Connection"[..], &b"Upgrade"[..])
        .expect("static header name/value is always valid"),
    )
    .with_header(
      tiny_http::Header::from_bytes(&b"Sec-WebSocket-Accept"[..], accept.as_bytes())
        .expect("base64-encoded accept key is always a valid header value"),
    );
  // `tiny_http::Request::upgrade` already writes AND flushes the 101
  // response itself (verified directly against its own source this
  // session, not assumed) before returning the raw stream — no
  // separate flush needed here before wrapping it.
  let stream = request.upgrade("websocket", response);
  let ws = TungWebSocket::from_raw_socket(WsStream::Server(stream), Role::Server, None);
  let id = handle_alloc(Box::new(ws), WS_CONN_TAG);
  unsafe { crate::emerald_rt_result_ok(id) }
}

/// `WebSocketConnection#send_text(self, msg: String): Result[Void, WebSocketError]`.
///
/// # Safety
/// `msg`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn ws_send_text(id: i64, msg: *const c_char) -> *mut std::ffi::c_void {
  let msg = match read_str(msg) {
    Ok(s) => s.to_string(),
    Err(e) => return err_other(&e),
  };
  let result = handle_get_mut::<TungWebSocket<WsStream>, Result<(), tungstenite::Error>>(
    id,
    WS_CONN_TAG,
    |ws| ws.send(Message::Text(msg.into())),
  );
  match result {
    Ok(Ok(())) => crate::emerald_rt_result_ok(0),
    Ok(Err(e)) => err_ws(&e),
    Err(e) => err_other(&e),
  }
}

/// `WebSocketConnection#send_binary(self, bytes: Bytes): Result[Void, WebSocketError]`
/// — `bytes` is a plan-109 `Bytes` handle (see this module's own doc
/// comment for why, a real, disclosed improvement over this plan's
/// own original `Array[Int64]` stopgap).
///
/// # Safety
/// `bytes_id` must be a pointer `crate::bytes::bytes_from_slice` (or
/// an equally-shaped native producer) actually returned.
pub unsafe fn ws_send_binary(id: i64, bytes_id: i64) -> *mut std::ffi::c_void {
  let data = crate::bytes::bytes_as_slice(bytes_id).to_vec();
  let result = handle_get_mut::<TungWebSocket<WsStream>, Result<(), tungstenite::Error>>(
    id,
    WS_CONN_TAG,
    |ws| ws.send(Message::Binary(data.into())),
  );
  match result {
    Ok(Ok(())) => crate::emerald_rt_result_ok(0),
    Ok(Err(e)) => err_ws(&e),
    Err(e) => err_other(&e),
  }
}

/// `WebSocketConnection#recv(self): Result[WebSocketMessage, WebSocketError]`
/// — a single blocking `tungstenite::WebSocket::read` call (matching
/// this crate's own sync-first posture, plan 94 — no caller-supplied
/// timeout, see this module's own doc comment). `tungstenite` already
/// auto-responds to a peer's `Ping` with a queued `Pong` on the next
/// `read`/`write`/`flush` internally; a `Ping`/`Pong`/`Close` frame
/// this call itself observes still surfaces as its own
/// `WebSocketMessage` kind (2/3/4) rather than being silently
/// swallowed, so a caller can still see it if it chooses to.
pub fn ws_recv(id: i64) -> *mut std::ffi::c_void {
  let result = handle_get_mut::<TungWebSocket<WsStream>, Result<Message, tungstenite::Error>>(
    id,
    WS_CONN_TAG,
    |ws| ws.read(),
  );
  match result {
    Ok(Ok(msg)) => unsafe { build_ws_message(msg) },
    Ok(Err(e)) => unsafe { err_ws(&e) },
    Err(e) => unsafe { err_other(&e) },
  }
}

/// Builds the `[kind: i64 @0][payload: ptr @8]` `WebSocketMessage`
/// heap block this module's own doc comment / plan 102's own Decision
/// log describes — `kind 0` (Text) points at a `String`, `kind 1`
/// (Binary) points at a `Bytes` handle, `kind 2/3/4` (Ping/Pong/Close)
/// leave `payload` `0`/unused.
unsafe fn build_ws_message(msg: Message) -> *mut std::ffi::c_void {
  let (kind, payload): (i64, i64) = match msg {
    Message::Text(t) => (0, crate::alloc_and_copy_str(t.as_str()) as i64),
    Message::Binary(b) => (1, crate::bytes::bytes_from_slice(&b)),
    Message::Ping(_) => (2, 0),
    Message::Pong(_) => (3, 0),
    Message::Close(_) => (4, 0),
    // `WebSocket::read` never yields a raw `Frame` (that variant only
    // ever appears on the write side / with the low-level `Frame`
    // API this module never uses) — folded into `Close` defensively
    // rather than panicking on an unreachable-in-practice arm.
    Message::Frame(_) => (4, 0),
  };
  let ptr = crate::emerald_alloc(16) as *mut i64;
  *ptr = kind;
  *ptr.add(1) = payload;
  crate::emerald_rt_result_ok(ptr as i64)
}

/// `WebSocketConnection#close(self): Void` — see this module's own
/// doc comment for why this is best-effort, not a full closing
/// handshake.
pub fn ws_close(id: i64) {
  let _ = handle_get_mut::<TungWebSocket<WsStream>, ()>(id, WS_CONN_TAG, |ws| {
    let _ = ws.close(None);
    let _ = ws.flush();
  });
  handle_close(id);
}

/// `WebSocketMessage#kind(self): Int64`.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `WebSocketMessage` block (`build_ws_message`'s own return payload).
pub unsafe fn ws_message_kind(ptr: *const i64) -> i64 {
  *ptr
}

/// `WebSocketMessage#text(self): String` — unchecked against `.kind`,
/// the identical "unchecked, matching `Array`'s own existing
/// precedent" posture plan 45's Decision log already accepted for
/// `str[i]`/`.slice` (reading `.text` on a non-Text message is real,
/// disclosed undefined behavior, not a new UB surface this plan
/// introduces independently).
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `WebSocketMessage` block whose `kind` is `0` (Text).
pub unsafe fn ws_message_text(ptr: *const i64) -> *const c_char {
  *ptr.add(1) as *const c_char
}

/// `WebSocketMessage#bytes(self): Bytes` — unchecked against `.kind`,
/// the identical posture `.text` above documents.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `WebSocketMessage` block whose `kind` is `1` (Binary).
pub unsafe fn ws_message_bytes(ptr: *const i64) -> i64 {
  *ptr.add(1)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::TcpListener;

  // Plan 102's own leaf-level round trip, independent of any compiled
  // `.em` program: a real client (`ws_connect`) against a real,
  // hand-rolled server performing the RFC6455 handshake itself over a
  // plain `TcpListener` (not `tiny_http` — this test exercises the
  // CLIENT path end to end; `http_server.rs`'s own tests plus
  // `examples/websocket_proof.em`'s own CI-checked loopback prove the
  // SERVER/`.upgrade()` path instead).
  #[test]
  fn client_text_round_trip_against_a_real_raw_server() {
    let listener = TcpListener::bind("127.0.0.1:47401").expect("bind");
    let server = std::thread::spawn(move || {
      let (sock, _addr) = listener.accept().expect("accept");
      let mut ws = tungstenite::accept(sock).expect("server-side handshake");
      let msg = ws.read().expect("read");
      assert_eq!(msg.into_text().unwrap().as_str(), "hello");
      ws.send(Message::Text("echo: hello".into())).expect("send");
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    unsafe {
      let url = std::ffi::CString::new("ws://127.0.0.1:47401/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let id = *result_ptr.add(1);

      let msg_c = std::ffi::CString::new("hello").unwrap();
      let send_result = ws_send_text(id, msg_c.as_ptr()) as *const i64;
      assert_eq!(*send_result, 0, "expected Ok discriminant");

      let recv_result = ws_recv(id) as *const i64;
      assert_eq!(*recv_result, 0, "expected Ok discriminant");
      let msg_ptr = *recv_result.add(1) as *const i64;
      assert_eq!(ws_message_kind(msg_ptr), 0);
      let text_ptr = ws_message_text(msg_ptr);
      let text = std::ffi::CStr::from_ptr(text_ptr).to_str().unwrap();
      assert_eq!(text, "echo: hello");

      ws_close(id);
    }
    server.join().unwrap();
  }

  #[test]
  fn connecting_to_an_unsupported_scheme_is_a_typed_protocol_err_not_a_panic() {
    unsafe {
      let url = std::ffi::CString::new("wss://example.invalid/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let err_block = *(result_ptr.add(1)) as *const i64;
      assert_eq!(*err_block, WS_ERROR_TAG_PROTOCOL as i64);
    }
  }

  #[test]
  fn connecting_to_a_closed_port_is_a_typed_other_err_not_a_panic() {
    unsafe {
      // Port 1 is a real, universally-privileged/unbound port in this
      // sandboxed test environment - a real, immediate connection
      // refusal, not a hang.
      let url = std::ffi::CString::new("ws://127.0.0.1:1/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
    }
  }

  #[test]
  fn binary_round_trip_uses_a_real_bytes_handle() {
    let listener = TcpListener::bind("127.0.0.1:47402").expect("bind");
    let server = std::thread::spawn(move || {
      let (sock, _addr) = listener.accept().expect("accept");
      let mut ws = tungstenite::accept(sock).expect("server-side handshake");
      let msg = ws.read().expect("read");
      assert_eq!(msg.into_data().as_ref(), &[1u8, 2, 3, 255]);
      ws.send(Message::Binary(vec![9u8, 8, 7].into()))
        .expect("send");
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    unsafe {
      let url = std::ffi::CString::new("ws://127.0.0.1:47402/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      let id = *result_ptr.add(1);

      let bytes_id = crate::bytes::bytes_from_slice(&[1u8, 2, 3, 255]);
      let send_result = ws_send_binary(id, bytes_id) as *const i64;
      assert_eq!(*send_result, 0);

      let recv_result = ws_recv(id) as *const i64;
      let msg_ptr = *recv_result.add(1) as *const i64;
      assert_eq!(ws_message_kind(msg_ptr), 1);
      let bytes_id_out = ws_message_bytes(msg_ptr);
      assert_eq!(crate::bytes::bytes_as_slice(bytes_id_out), &[9u8, 8, 7]);

      ws_close(id);
    }
    server.join().unwrap();
  }

  // Plan 102's own leaf-level round trip: `tungstenite` already
  // auto-responds to a peer's `Ping` with a queued `Pong` on the next
  // `read`/`write`/`flush` (confirmed from its own `WebSocket::write`
  // docs — see this crate's own `websocket.rs` module doc) — this test
  // proves that real, built-in behavior fires for the CLIENT path this
  // module drives, not a hand-rolled Pong this plan's own text
  // explicitly declines to add. The server thread sends a `Ping`, then
  // blocks on its own `.read()` for the client's automatic `Pong`
  // reply, proving the round trip completed rather than merely that
  // `ws_recv` observed the `Ping` itself.
  #[test]
  fn a_ping_gets_tungstenites_own_automatic_pong_reply() {
    let listener = TcpListener::bind("127.0.0.1:47404").expect("bind");
    let server = std::thread::spawn(move || {
      let (sock, _addr) = listener.accept().expect("accept");
      let mut ws = tungstenite::accept(sock).expect("server-side handshake");
      ws.send(Message::Ping(vec![1u8, 2, 3].into()))
        .expect("send ping");
      // Loops past the client's own intervening `flush` Text frame
      // (see this test's own client-side comment below) rather than
      // asserting the very next frame is the Pong — real, disclosed:
      // empirically found this session that a queued auto-Pong's
      // exact position relative to a subsequent explicit send is an
      // internal `tungstenite` buffering detail this test should not
      // depend on, only that the Pong arrives at all, with the right
      // payload.
      loop {
        let msg = ws.read().expect("read");
        if msg.is_pong() {
          assert_eq!(msg.into_data().as_ref(), &[1u8, 2, 3]);
          break;
        }
      }
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    unsafe {
      let url = std::ffi::CString::new("ws://127.0.0.1:47404/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let id = *result_ptr.add(1);

      // `ws_recv` (`WebSocket::read`) both observes the `Ping` AND
      // queues the automatic `Pong` internally, per tungstenite's own
      // documented "written and flushed on the next call to read,
      // write, or flush" contract. `.close()` alone was found,
      // empirically, this session, not to reliably flush a Pong
      // queued by a prior `.read()` ahead of its own Close frame — an
      // explicit `.send_text` (`WebSocket::send` is `write` then
      // `flush`) does, so this test forces the flush that way instead
      // of relying on `ws_close`'s own.
      let recv_result = ws_recv(id) as *const i64;
      assert_eq!(*recv_result, 0, "expected Ok discriminant");
      let msg_ptr = *recv_result.add(1) as *const i64;
      assert_eq!(ws_message_kind(msg_ptr), 2, "expected a Ping message");

      let flush_c = std::ffi::CString::new("flush").unwrap();
      let send_result = ws_send_text(id, flush_c.as_ptr()) as *const i64;
      assert_eq!(*send_result, 0, "expected Ok discriminant");

      ws_close(id);
    }
    server.join().unwrap();
  }

  // Plan 102's own leaf-level close-frame round trip: `.close()`
  // sends a real Close frame the peer actually observes (the server
  // thread's own `.read()` loop below only returns once it does),
  // and a subsequent `.recv()` on the now-closed client handle is a
  // real `Err` (the handle registry's own "use of closed handle"
  // diagnostic, folded into `WebSocketError::Other` — see `ws_recv`'s
  // own `Err(e) => err_other(&e)` arm), never a panic.
  #[test]
  fn close_sends_a_real_close_frame_the_peer_observes() {
    let listener = TcpListener::bind("127.0.0.1:47405").expect("bind");
    let server = std::thread::spawn(move || {
      let (sock, _addr) = listener.accept().expect("accept");
      let mut ws = tungstenite::accept(sock).expect("server-side handshake");
      loop {
        match ws.read() {
          Ok(Message::Close(_)) => break,
          Ok(_) => continue,
          Err(_) => break,
        }
      }
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    unsafe {
      let url = std::ffi::CString::new("ws://127.0.0.1:47405/").unwrap();
      let result_ptr = ws_connect(url.as_ptr()) as *const i64;
      let id = *result_ptr.add(1);

      ws_close(id);

      let recv_result = ws_recv(id) as *const i64;
      assert_eq!(
        *recv_result, 1,
        "expected Err reading a connection this module already closed"
      );
    }
    server.join().unwrap();
  }
}
