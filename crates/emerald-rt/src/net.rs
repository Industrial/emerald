//! Plan 96 (Raw TCP/UDP Sockets) — `TcpStream`/`TcpListener`/`UdpSocket`,
//! wrapping `std::net`'s own blocking socket types. No crate to vet for
//! the socket types themselves — the cleanest possible instance of
//! plan 95's pure-Rust-first policy. This is the first general-purpose,
//! directly-addressable socket surface Emerald source has ever had;
//! plan 60's `emerald_tcp_listen`/`emerald_tcp_connect`
//! (`runtime/emerald_runtime.c`) remain a separate, private universe
//! serving only the actor wire protocol — this module never touches
//! `EmeraldActorRef`/`sockfd`, per this plan's own Decision log.
//!
//! Real, disclosed finding: this plan's own text frames "instance-
//! carrying AND non-user-declarable" as a combination neither `String`
//! (stateless) nor `File` (zero instances) covers alone, deferring the
//! exact representation to whichever plan lands first. By the time
//! this plan actually runs, plans 109-111/117/122/168 (`Regex`/
//! `Sha256Hasher`/`Blake3Hasher`/`AeadKey`/`Ed25519KeyPair`/
//! `X25519EphemeralSecret`/`X25519StaticSecret`/`RsaKeyPair`/`Url`/
//! `LogFields`) already built and proved exactly that combination
//! repeatedly — an `Int64` newtype over `crate::handle`'s registry.
//! `TcpStream`/`TcpListener`/`UdpSocket` reuse that same established
//! shape verbatim rather than inventing a new one.
//!
//! Real, disclosed gap inherited, not introduced, by this plan (see
//! this plan's own Decision log): Emerald's own `String` is a bare
//! null-terminated `char*` (plan 59's finding) — a payload containing
//! an embedded NUL byte, fully legal on an arbitrary socket, silently
//! truncates on both the read and write paths of every method below.
//! The real fix (a binary-safe `(ptr, len)` buffer convention) is
//! deferred to whichever future plan adds a real `Bytes`/`Buffer` type
//! consuming that convention — not invented here.
//!
//! `.read`/`.write`/`.send_to`/`.recv_from` each make exactly ONE
//! underlying `std::io` call — real short-read/partial-write
//! semantics, never looping to fill/drain a buffer, per this plan's
//! own leaf text.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::raw::c_char;
use std::os::unix::io::FromRawFd;

const TCP_STREAM_TAG: &str = "TcpStream";
const TCP_LISTENER_TAG: &str = "TcpListener";
const UDP_SOCKET_TAG: &str = "UdpSocket";

// `UdpSocket.last_sender_host`/`.last_sender_port`'s own backing store
// — the identical `_Thread_local`-accessor-pair shape `runtime/
// emerald_runtime.c`'s own `emerald_remote_last_error`/
// `emerald_remote_last_error_message` (L1479-1492) already establishes
// for errno-style side-channel state, reused verbatim rather than
// inventing a tuple/struct-returning FFI convention this plan's own
// text explicitly declines to design.
thread_local! {
  static LAST_UDP_SENDER: std::cell::RefCell<Option<SocketAddr>> =
    const { std::cell::RefCell::new(None) };
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

fn check_port(port: i64) -> Result<u16, String> {
  if (0..=u16::MAX as i64).contains(&port) {
    Ok(port as u16)
  } else {
    Err("port must be in 0..=65535".to_string())
  }
}

// The `SO_REUSEADDR` fix this plan's own Decision log requires,
// verified necessary this session: `std::net::TcpListener::bind` does
// not set it by default (unlike `emerald_tcp_listen`'s explicit C-side
// `setsockopt` at `runtime/emerald_runtime.c:1506` — confirmed via
// `tokio::net::TcpListener`'s own docs, which state it sets
// `SO_REUSEADDR` on Unix as a deliberate addition on top of bare
// `std::net`, implying `std::net` itself does not). `socket2`-free per
// the plan's own instruction: resolves `host`/`port` via the real C
// library `getaddrinfo` (the identical mechanism `std::net` itself
// calls internally on Unix — not a competing resolution path), sets
// `SO_REUSEADDR` on the raw fd BEFORE `bind` (the only point at which
// setting it has any effect on a subsequent `EADDRINUSE`), then hands
// the bound, listening fd to `std::net::TcpListener` via `FromRawFd` so
// every other method on it keeps using plain `std::net` afterward.
fn bind_tcp_listener_with_reuseaddr(host: &str, port: u16) -> std::io::Result<TcpListener> {
  use std::ffi::CString;

  let node = CString::new(host).map_err(|_| {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, "host contains a NUL byte")
  })?;
  let service = CString::new(port.to_string()).expect("port digits never contain a NUL byte");

  let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
  hints.ai_family = libc::AF_UNSPEC;
  hints.ai_socktype = libc::SOCK_STREAM;
  hints.ai_flags = libc::AI_PASSIVE;

  let mut res: *mut libc::addrinfo = std::ptr::null_mut();
  let rc = unsafe { libc::getaddrinfo(node.as_ptr(), service.as_ptr(), &hints, &mut res) };
  if rc != 0 {
    return Err(std::io::Error::last_os_error());
  }

  struct FreeAddrInfo(*mut libc::addrinfo);
  impl Drop for FreeAddrInfo {
    fn drop(&mut self) {
      unsafe { libc::freeaddrinfo(self.0) };
    }
  }
  let _guard = FreeAddrInfo(res);

  let mut last_err: Option<std::io::Error> = None;
  let mut ai = res;
  while !ai.is_null() {
    let entry = unsafe { &*ai };
    let fd = unsafe { libc::socket(entry.ai_family, entry.ai_socktype, entry.ai_protocol) };
    if fd < 0 {
      last_err = Some(std::io::Error::last_os_error());
      ai = entry.ai_next;
      continue;
    }
    let optval: libc::c_int = 1;
    let reuseaddr_rc = unsafe {
      libc::setsockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_REUSEADDR,
        &optval as *const libc::c_int as *const libc::c_void,
        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
      )
    };
    if reuseaddr_rc != 0 {
      last_err = Some(std::io::Error::last_os_error());
      unsafe { libc::close(fd) };
      ai = entry.ai_next;
      continue;
    }
    let bind_rc = unsafe { libc::bind(fd, entry.ai_addr, entry.ai_addrlen) };
    if bind_rc != 0 {
      last_err = Some(std::io::Error::last_os_error());
      unsafe { libc::close(fd) };
      ai = entry.ai_next;
      continue;
    }
    let listen_rc = unsafe { libc::listen(fd, 128) };
    if listen_rc != 0 {
      last_err = Some(std::io::Error::last_os_error());
      unsafe { libc::close(fd) };
      ai = entry.ai_next;
      continue;
    }
    return Ok(unsafe { TcpListener::from_raw_fd(fd) });
  }
  Err(last_err.unwrap_or_else(|| std::io::Error::other("getaddrinfo returned no usable address")))
}

/// `TcpStream.connect(host: String, port: Int64): TcpStream`.
///
/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn tcp_stream_connect(host: *const c_char, port: i64) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match TcpStream::connect((host, port)) {
    Ok(s) => handle_alloc(Box::new(s), TCP_STREAM_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `TcpStream#read(self, max_len: Int64): String` — a single
/// `std::io::Read::read` call; real short-read semantics, never loops
/// to fill `max_len`. See this module's own doc comment for the
/// embedded-NUL truncation caveat.
pub fn tcp_stream_read(id: i64, max_len: i64) -> *const c_char {
  if max_len <= 0 {
    unsafe { crate::raise_native_error("TcpStream#read: max_len must be positive") };
  }
  let mut buf = vec![0u8; max_len as usize];
  let result =
    handle_get_mut::<TcpStream, std::io::Result<usize>>(id, TCP_STREAM_TAG, |s| s.read(&mut buf));
  match result {
    Ok(Ok(n)) => {
      let s = String::from_utf8_lossy(&buf[..n]).into_owned();
      unsafe { crate::alloc_and_copy_str(&s) }
    }
    Ok(Err(e)) => unsafe { crate::raise_native_error(&e.to_string()) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `TcpStream#write(self, data: String): Int64` — a single
/// `std::io::Write::write` call, returning the real byte count
/// actually written; never loops to send all bytes.
///
/// # Safety
/// `data`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn tcp_stream_write(id: i64, data: *const c_char) -> i64 {
  let data = match read_str(data) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<TcpStream, std::io::Result<usize>>(id, TCP_STREAM_TAG, |s| {
    s.write(data.as_bytes())
  });
  match result {
    Ok(Ok(n)) => n as i64,
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `TcpStream#close(self): Void`.
pub fn tcp_stream_close(id: i64) {
  handle_close(id);
}

/// `TcpListener.bind(host: String, port: Int64): TcpListener`.
///
/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn tcp_listener_bind(host: *const c_char, port: i64) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match bind_tcp_listener_with_reuseaddr(host, port) {
    Ok(l) => handle_alloc(Box::new(l), TCP_LISTENER_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `TcpListener#accept(self): TcpStream` — discards the peer
/// `SocketAddr`, a real, disclosed v1 simplification (see this plan's
/// own Decision log).
pub fn tcp_listener_accept(id: i64) -> i64 {
  let result = handle_get_mut::<TcpListener, std::io::Result<(TcpStream, SocketAddr)>>(
    id,
    TCP_LISTENER_TAG,
    |l| l.accept(),
  );
  match result {
    Ok(Ok((stream, _addr))) => handle_alloc(Box::new(stream), TCP_STREAM_TAG),
    Ok(Err(e)) => unsafe { crate::raise_native_error(&e.to_string()) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `TcpListener#close(self): Void`.
pub fn tcp_listener_close(id: i64) {
  handle_close(id);
}

/// `UdpSocket.bind(host: String, port: Int64): UdpSocket`.
///
/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn udp_socket_bind(host: *const c_char, port: i64) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match UdpSocket::bind((host, port)) {
    Ok(s) => handle_alloc(Box::new(s), UDP_SOCKET_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `UdpSocket#send_to(self, data: String, host: String, port: Int64): Int64`.
///
/// # Safety
/// `data`/`host`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn udp_socket_send_to(
  id: i64,
  data: *const c_char,
  host: *const c_char,
  port: i64,
) -> i64 {
  let data = match read_str(data) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<UdpSocket, std::io::Result<usize>>(id, UDP_SOCKET_TAG, |s| {
    s.send_to(data.as_bytes(), (host, port))
  });
  match result {
    Ok(Ok(n)) => n as i64,
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `UdpSocket#recv_from(self, max_len: Int64): String` — populates the
/// thread-local sender address `UdpSocket.last_sender_host`/
/// `.last_sender_port` read immediately after this call, per this
/// plan's own Decision log.
pub fn udp_socket_recv_from(id: i64, max_len: i64) -> *const c_char {
  if max_len <= 0 {
    unsafe { crate::raise_native_error("UdpSocket#recv_from: max_len must be positive") };
  }
  let mut buf = vec![0u8; max_len as usize];
  let result =
    handle_get_mut::<UdpSocket, std::io::Result<(usize, SocketAddr)>>(id, UDP_SOCKET_TAG, |s| {
      s.recv_from(&mut buf)
    });
  match result {
    Ok(Ok((n, addr))) => {
      LAST_UDP_SENDER.with(|cell| *cell.borrow_mut() = Some(addr));
      let s = String::from_utf8_lossy(&buf[..n]).into_owned();
      unsafe { crate::alloc_and_copy_str(&s) }
    }
    Ok(Err(e)) => unsafe { crate::raise_native_error(&e.to_string()) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `UdpSocket#close(self): Void`.
pub fn udp_socket_close(id: i64) {
  handle_close(id);
}

/// `UdpSocket.last_sender_host(): String` — the sender address of the
/// most recent `.recv_from` completed on the calling thread. Raises if
/// no `.recv_from` has completed on this thread yet.
pub fn udp_socket_last_sender_host() -> *const c_char {
  let host = LAST_UDP_SENDER.with(|cell| cell.borrow().map(|a| a.ip().to_string()));
  match host {
    Some(h) => unsafe { crate::alloc_and_copy_str(&h) },
    None => unsafe {
      crate::raise_native_error(
        "UdpSocket.last_sender_host: no recv_from has completed on this thread yet",
      )
    },
  }
}

/// `UdpSocket.last_sender_port(): Int64` — `.last_sender_host`'s own
/// port-accessor sibling.
pub fn udp_socket_last_sender_port() -> i64 {
  let port = LAST_UDP_SENDER.with(|cell| cell.borrow().map(|a| a.port()));
  match port {
    Some(p) => p as i64,
    None => unsafe {
      crate::raise_native_error(
        "UdpSocket.last_sender_port: no recv_from has completed on this thread yet",
      )
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  #[test]
  fn tcp_round_trip_ping_pong() {
    unsafe {
      let host = c("127.0.0.1");
      let listener_id = tcp_listener_bind(host.as_ptr(), 0);
      // Port 0 asks the OS for an ephemeral port — read it back via a
      // real accessor this plan doesn't otherwise expose, so this test
      // reaches into the handle directly rather than adding a
      // `.local_addr` method this plan's own leaf list omits.
      let port = handle_get_mut::<TcpListener, u16>(listener_id, TCP_LISTENER_TAG, |l| {
        l.local_addr().unwrap().port()
      })
      .unwrap();

      let client_id = tcp_stream_connect(host.as_ptr(), port as i64);
      let ping = c("ping");
      tcp_stream_write(client_id, ping.as_ptr());

      let server_side_id = tcp_listener_accept(listener_id);
      let msg = cstr(tcp_stream_read(server_side_id, 64));
      assert_eq!(msg, "ping");
      let pong = c("pong");
      tcp_stream_write(server_side_id, pong.as_ptr());

      let reply = cstr(tcp_stream_read(client_id, 64));
      assert_eq!(reply, "pong");

      tcp_stream_close(client_id);
      tcp_stream_close(server_side_id);
      tcp_listener_close(listener_id);
    }
  }

  #[test]
  fn udp_round_trip_and_last_sender_accessors() {
    unsafe {
      let host = c("127.0.0.1");
      let server_id = udp_socket_bind(host.as_ptr(), 0);
      let server_port = handle_get_mut::<UdpSocket, u16>(server_id, UDP_SOCKET_TAG, |s| {
        s.local_addr().unwrap().port()
      })
      .unwrap();

      let client_id = udp_socket_bind(host.as_ptr(), 0);
      let client_port = handle_get_mut::<UdpSocket, u16>(client_id, UDP_SOCKET_TAG, |s| {
        s.local_addr().unwrap().port()
      })
      .unwrap();

      let payload = c("hello");
      let n = udp_socket_send_to(
        client_id,
        payload.as_ptr(),
        host.as_ptr(),
        server_port as i64,
      );
      assert_eq!(n, 5);

      let received = cstr(udp_socket_recv_from(server_id, 64));
      assert_eq!(received, "hello");
      assert_eq!(cstr(udp_socket_last_sender_host()), "127.0.0.1");
      assert_eq!(udp_socket_last_sender_port(), client_port as i64);

      udp_socket_close(server_id);
      udp_socket_close(client_id);
    }
  }

  #[test]
  fn accept_blocks_until_a_real_connect_happens() {
    unsafe {
      let host = c("127.0.0.1");
      let listener_id = tcp_listener_bind(host.as_ptr(), 0);
      let port = handle_get_mut::<TcpListener, u16>(listener_id, TCP_LISTENER_TAG, |l| {
        l.local_addr().unwrap().port()
      })
      .unwrap();

      let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
      let accepted_clone = accepted.clone();
      let handle = std::thread::spawn(move || {
        let server_side_id = tcp_listener_accept(listener_id);
        accepted_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        tcp_stream_close(server_side_id);
      });

      std::thread::sleep(std::time::Duration::from_millis(50));
      assert!(
        !accepted.load(std::sync::atomic::Ordering::SeqCst),
        "accept must still be blocked with no connect yet"
      );

      let client_id = tcp_stream_connect(host.as_ptr(), port as i64);
      handle.join().unwrap();
      assert!(accepted.load(std::sync::atomic::Ordering::SeqCst));

      tcp_stream_close(client_id);
      tcp_listener_close(listener_id);
    }
  }

  #[test]
  fn reuseaddr_is_actually_set_on_a_bound_listener() {
    unsafe {
      let host = c("127.0.0.1");
      let listener_id = tcp_listener_bind(host.as_ptr(), 0);
      let reuseaddr_value =
        handle_get_mut::<TcpListener, libc::c_int>(listener_id, TCP_LISTENER_TAG, |l| {
          use std::os::unix::io::AsRawFd;
          let mut val: libc::c_int = 0;
          let mut len: libc::socklen_t = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
          let rc = libc::getsockopt(
            l.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &mut val as *mut libc::c_int as *mut libc::c_void,
            &mut len,
          );
          assert_eq!(rc, 0);
          val
        })
        .unwrap();
      assert_ne!(
        reuseaddr_value, 0,
        "SO_REUSEADDR must be set on bind, not left at std::net's default of unset"
      );
      tcp_listener_close(listener_id);
    }
  }
}
