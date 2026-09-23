//! Plan 99 (TLS) — `Tls.connect`/`.connect_with_roots`/`.listen`,
//! wrapping `rustls` (plus `rustls-native-certs`/`webpki-roots` for
//! root-certificate sources and `rustls-pemfile` for PEM cert/key
//! loading), layered directly on top of `std::net::{TcpStream,
//! TcpListener}` — never opening a socket of its own, per this plan's
//! own Decision log. `rustls::StreamOwned` is genuinely synchronous
//! (`rustls`'s own core protocol state machine is "sans-IO"; `StreamOwned`
//! is its official blocking `io::Read`/`io::Write` adapter), so this
//! module needs none of plan 94's async-bridging machinery at all —
//! the strongest case in this whole session for that pattern's own
//! "prefer a crate's sync API" preference.
//!
//! **Real, disclosed representational choice**: `TlsStream` must be
//! able to hold EITHER a client-side (`rustls::ClientConnection`) or a
//! server-side (`rustls::ServerConnection`) `StreamOwned` under one
//! Emerald-facing type (`Tls.connect` produces the former,
//! `TlsListener#accept` the latter) — unlike `TcpStream`/`TcpListener`
//! (plan 96), which never mix two distinct backing Rust types under a
//! single `crate::handle` tag. `TlsStreamInner` is a small enum over
//! both `StreamOwned` variants, implementing `std::io::Read`/`Write`
//! by dispatching to whichever variant is actually stored — the
//! `crate::handle` registry then holds exactly one `TlsStreamInner`
//! per `TlsStream` handle, the same "one boxed value per id" shape
//! every other handle-carrying type in this crate already uses.
//!
//! Per this plan's own Decision log, a `TlsStream` closing also closes
//! its underlying TCP connection (`rustls::StreamOwned` takes its
//! socket by value; there is no supported way to hand back a live,
//! un-TLS-wrapped `TcpStream` afterward) — `.close()` therefore has no
//! "keep the raw socket open" mode, inherited from the wrapped crate's
//! own ownership shape, not introduced here.
//!
//! The TLS handshake itself completes eagerly, inside `Tls.connect`/
//! `.connect_with_roots`/`TlsListener#accept` themselves (via a
//! `complete_io` loop while `is_handshaking()`), not lazily on the
//! first `.read`/`.write` — a certificate-validation or handshake
//! failure therefore surfaces at connect/accept time, matching this
//! plan's own leaf text ("performs the TLS handshake... and exposes
//! `.read`/`.write`/`.close`", listing the handshake as something
//! `Tls.connect` itself does, not something deferred).

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{
  ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::raw::c_char;
use std::sync::{Arc, OnceLock};

const TLS_STREAM_TAG: &str = "TlsStream";
const TLS_LISTENER_TAG: &str = "TlsListener";

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

// This plan's own disclosed representational choice (see this
// module's own doc comment above) — one `TlsStream` handle can hold
// either backing `StreamOwned` shape, dispatched by hand since
// `ClientConnection`/`ServerConnection` are two distinct, unrelated
// `rustls` types with no shared `Read`/`Write`-capable supertype this
// crate's own dependency version exposes.
enum TlsStreamInner {
  Client(StreamOwned<ClientConnection, TcpStream>),
  Server(StreamOwned<ServerConnection, TcpStream>),
}

impl Read for TlsStreamInner {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    match self {
      TlsStreamInner::Client(s) => s.read(buf),
      TlsStreamInner::Server(s) => s.read(buf),
    }
  }
}

impl Write for TlsStreamInner {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    match self {
      TlsStreamInner::Client(s) => s.write(buf),
      TlsStreamInner::Server(s) => s.write(buf),
    }
  }
  fn flush(&mut self) -> std::io::Result<()> {
    match self {
      TlsStreamInner::Client(s) => s.flush(),
      TlsStreamInner::Server(s) => s.flush(),
    }
  }
}

// `TlsListener#accept`'s own handle-table entry — the plan-96
// `TcpListener` plus the `rustls::ServerConfig` every accepted
// connection's own handshake is built from (loaded once, at
// `Tls.listen` time, from the caller's PEM cert/key files).
struct TlsListenerData {
  listener: TcpListener,
  config: Arc<ServerConfig>,
}

// `rustls` 0.23 requires a process-level `CryptoProvider` installed
// before `ClientConfig::builder()`/`ServerConfig::builder()` can be
// called at all — installed exactly once, lazily, the same
// `std::sync::Once` shape this crate's own panic-hook installer
// (`lib.rs::install_panic_hook_once`) already establishes. `ring` is
// this crate's own already-established C-exception backend (plan 97's
// Decision log prefers it over `aws-lc-rs` for DNS-over-TLS; this
// plan reuses that same choice rather than introducing a second one).
fn ensure_crypto_provider_installed() {
  static INIT: std::sync::Once = std::sync::Once::new();
  INIT.call_once(|| {
    let _ = rustls::crypto::ring::default_provider().install_default();
  });
}

// `leaf-root-cert-source-decision`: the OS's own configured trust
// store, loaded once, lazily, the first time it's actually needed —
// never eagerly at process start (a program that never calls
// `Tls.connect` pays nothing for this).
fn native_root_store() -> Result<RootCertStore, String> {
  static CACHE: OnceLock<Result<RootCertStore, String>> = OnceLock::new();
  CACHE
    .get_or_init(|| {
      let mut roots = RootCertStore::empty();
      let result = rustls_native_certs::load_native_certs();
      for cert in result.certs {
        roots.add(cert).map_err(|e| e.to_string())?;
      }
      if roots.is_empty() {
        return Err(format!(
          "no usable root certificates found in the OS trust store ({} error(s) while loading it)",
          result.errors.len()
        ));
      }
      Ok(roots)
    })
    .clone()
}

// `leaf-root-cert-source-decision`'s explicit opt-in alternative — the
// build-time-bundled Mozilla root set, for a caller that specifically
// wants redeploy-independent trust rather than whatever the deploying
// OS happens to trust.
fn webpki_root_store() -> RootCertStore {
  static CACHE: OnceLock<RootCertStore> = OnceLock::new();
  CACHE
    .get_or_init(|| RootCertStore {
      roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    })
    .clone()
}

// Shared client-handshake logic every `Tls.connect*` entry point (and
// this module's own local, network-independent test) funnels through.
fn connect_with_root_store(
  host: &str,
  port: u16,
  roots: RootCertStore,
) -> Result<TlsStreamInner, String> {
  ensure_crypto_provider_installed();
  let sock = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
  let config = Arc::new(
    ClientConfig::builder()
      .with_root_certificates(roots)
      .with_no_client_auth(),
  );
  let server_name = ServerName::try_from(host.to_string())
    .map_err(|e| format!("invalid TLS server name `{host}`: {e}"))?;
  let conn = ClientConnection::new(config, server_name).map_err(|e| e.to_string())?;
  let mut stream = StreamOwned::new(conn, sock);
  while stream.conn.is_handshaking() {
    stream
      .conn
      .complete_io(&mut stream.sock)
      .map_err(|e| e.to_string())?;
  }
  Ok(TlsStreamInner::Client(stream))
}

// Test-only entry point, deliberately NOT reachable from the Emerald-
// facing FFI surface below: `Tls.connect_with_roots` only ever exposes
// the two-mode (`"native"`/`"webpki"`) dispatch this plan's own
// Decision log states, never an arbitrary `RootCertStore` literal —
// this function exists solely so `leaf-tests-and-example`'s own local,
// network-independent round trip can trust a freshly generated
// self-signed test certificate directly as an explicit anchor, with no
// file-path/mode-string plumbing this plan's public surface has no
// other reason to carry.
#[cfg(test)]
fn connect_with_explicit_roots_for_test(
  host: &str,
  port: u16,
  roots: RootCertStore,
) -> Result<TlsStreamInner, String> {
  connect_with_root_store(host, port, roots)
}

fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, String> {
  let file = std::fs::File::open(path)
    .map_err(|e| format!("failed to open certificate file `{path}`: {e}"))?;
  let mut reader = std::io::BufReader::new(file);
  rustls_pemfile::certs(&mut reader)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("failed to parse PEM certificate chain in `{path}`: {e}"))
}

fn load_key(path: &str) -> Result<PrivateKeyDer<'static>, String> {
  let file = std::fs::File::open(path)
    .map_err(|e| format!("failed to open private key file `{path}`: {e}"))?;
  let mut reader = std::io::BufReader::new(file);
  rustls_pemfile::private_key(&mut reader)
    .map_err(|e| format!("failed to parse PEM private key in `{path}`: {e}"))?
    .ok_or_else(|| format!("no private key found in `{path}`"))
}

fn listen_impl(
  host: &str,
  port: u16,
  cert_path: &str,
  key_path: &str,
) -> Result<TlsListenerData, String> {
  ensure_crypto_provider_installed();
  let listener = TcpListener::bind((host, port)).map_err(|e| e.to_string())?;
  let certs = load_certs(cert_path)?;
  let key = load_key(key_path)?;
  let config = ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| e.to_string())?;
  Ok(TlsListenerData {
    listener,
    config: Arc::new(config),
  })
}

// `leaf-tls-server-listen-and-accept`: mutual-TLS/client-cert
// verification is out of scope (`with_no_client_auth` above) — a
// client presenting no certificate is expected and accepted.
fn accept_impl(data: &mut TlsListenerData) -> Result<TlsStreamInner, String> {
  let (sock, _addr) = data.listener.accept().map_err(|e| e.to_string())?;
  let conn = ServerConnection::new(data.config.clone()).map_err(|e| e.to_string())?;
  let mut stream = StreamOwned::new(conn, sock);
  while stream.conn.is_handshaking() {
    stream
      .conn
      .complete_io(&mut stream.sock)
      .map_err(|e| e.to_string())?;
  }
  Ok(TlsStreamInner::Server(stream))
}

/// `Tls.connect(host: String, port: Int64): TlsStream` — opens a
/// plan-96 `TcpStream`, wraps it in a `rustls` client handshake
/// against the OS's own native root store, raising plan 92's canonical
/// error on any failure (connect, handshake, or certificate
/// validation).
///
/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn tls_connect(host: *const c_char, port: i64) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let roots = match native_root_store() {
    Ok(r) => r,
    Err(e) => crate::raise_native_error(&e),
  };
  match connect_with_root_store(host, port, roots) {
    Ok(stream) => handle_alloc(Box::new(stream), TLS_STREAM_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Tls.connect_with_roots(host: String, port: Int64, mode: String): TlsStream`
/// — `mode` is `"native"` (identical to `Tls.connect`, spelled out
/// explicitly) or `"webpki"` (the build-time-bundled Mozilla root
/// set). Any other `mode` raises.
///
/// # Safety
/// `host`/`mode`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn tls_connect_with_roots(host: *const c_char, port: i64, mode: *const c_char) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let mode = match read_str(mode) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let roots = match mode {
    "native" => native_root_store(),
    "webpki" => Ok(webpki_root_store()),
    other => Err(format!(
      "Tls.connect_with_roots: unknown mode `{other}` (expected \"native\" or \"webpki\")"
    )),
  };
  let roots = match roots {
    Ok(r) => r,
    Err(e) => crate::raise_native_error(&e),
  };
  match connect_with_root_store(host, port, roots) {
    Ok(stream) => handle_alloc(Box::new(stream), TLS_STREAM_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `TlsStream#read(self, max_len: Int64): String` — a single
/// underlying `read` call, the identical short-read semantics plan
/// 96's `TcpStream#read` already discloses (inherited verbatim from
/// the `std::io::Read` implementation `StreamOwned` itself provides).
pub fn tls_stream_read(id: i64, max_len: i64) -> *const c_char {
  if max_len <= 0 {
    unsafe { crate::raise_native_error("TlsStream#read: max_len must be positive") };
  }
  let mut buf = vec![0u8; max_len as usize];
  let result = handle_get_mut::<TlsStreamInner, std::io::Result<usize>>(id, TLS_STREAM_TAG, |s| {
    s.read(&mut buf)
  });
  match result {
    Ok(Ok(n)) => {
      let s = String::from_utf8_lossy(&buf[..n]).into_owned();
      unsafe { crate::alloc_and_copy_str(&s) }
    }
    Ok(Err(e)) => unsafe { crate::raise_native_error(&e.to_string()) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `TlsStream#write(self, data: String): Int64` — a single underlying
/// `write` call, the identical partial-write semantics plan 96's
/// `TcpStream#write` already discloses.
///
/// # Safety
/// `data`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn tls_stream_write(id: i64, data: *const c_char) -> i64 {
  let data = match read_str(data) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<TlsStreamInner, std::io::Result<usize>>(id, TLS_STREAM_TAG, |s| {
    s.write(data.as_bytes())
  });
  match result {
    Ok(Ok(n)) => n as i64,
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `TlsStream#close(self): Void` — per this plan's own Decision log,
/// also closes the underlying TCP connection; there is no "keep the
/// raw socket open" mode (see this module's own doc comment).
pub fn tls_stream_close(id: i64) {
  handle_close(id);
}

/// `Tls.listen(host: String, port: Int64, cert_path: String, key_path: String): TlsListener`
/// — opens a plan-96 `TcpListener`, loads a PEM certificate chain and
/// private key from `cert_path`/`key_path` via `rustls-pemfile`, and
/// builds a `rustls::ServerConfig`. Certificate/key loading failures
/// raise plan 92's canonical error shape with the real `rustls-pemfile`
/// error text.
///
/// # Safety
/// `host`/`cert_path`/`key_path`, if non-null, must point to valid,
/// NUL-terminated C strings.
pub unsafe fn tls_listen(
  host: *const c_char,
  port: i64,
  cert_path: *const c_char,
  key_path: *const c_char,
) -> i64 {
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let port = match check_port(port) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let cert_path = match read_str(cert_path) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let key_path = match read_str(key_path) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match listen_impl(host, port, cert_path, key_path) {
    Ok(data) => handle_alloc(Box::new(data), TLS_LISTENER_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `TlsListener#accept(self): TlsStream` — a blocking `TcpListener::accept`
/// followed by a blocking, server-side `rustls` handshake. A client
/// presenting no certificate (mutual TLS is out of scope, see this
/// module's own doc comment) or a malformed handshake both raise plan
/// 92's canonical error shape with the real `rustls` error text.
pub fn tls_listener_accept(id: i64) -> i64 {
  let result = handle_get_mut::<TlsListenerData, Result<TlsStreamInner, String>>(
    id,
    TLS_LISTENER_TAG,
    accept_impl,
  );
  match result {
    Ok(Ok(stream)) => handle_alloc(Box::new(stream), TLS_STREAM_TAG),
    Ok(Err(e)) => unsafe { crate::raise_native_error(&e) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `TlsListener#close(self): Void`.
pub fn tls_listener_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;
  use rcgen::{generate_simple_self_signed, CertifiedKey};
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  // `leaf-tests-and-example`'s network-dependent half — a real client
  // handshake against a known-stable public HTTPS host, gated the same
  // (ungated) way plan 97's own DNS test and plan 100's own HTTP
  // client tests already are, not invented fresh here. `example.com`
  // is IANA's own reserved, stable-by-design example domain, matching
  // this plan's own Concrete Proof.
  #[test]
  fn connect_against_a_real_public_host_completes_a_real_handshake() {
    unsafe {
      let host = c("example.com");
      let id = tls_connect(host.as_ptr(), 443);
      let req = c("GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n");
      let n = tls_stream_write(id, req.as_ptr());
      assert!(n > 0, "expected a positive byte count from #write");
      let resp = cstr(tls_stream_read(id, 64));
      assert!(
        resp.starts_with("HTTP/1."),
        "expected a well-formed HTTP status line, got {resp:?}"
      );
      tls_stream_close(id);
    }
  }

  // `leaf-tests-and-example`'s network-independent half — a real
  // `Tls.listen`/`TlsListener#accept`/`Tls.connect`-shaped round trip
  // entirely within this test process, using a freshly generated
  // self-signed certificate as the client's own explicit trust anchor
  // (`connect_with_explicit_roots_for_test`, never the OS trust
  // store), proving the full handshake/encryption/byte-transport path
  // with no network dependency at all.
  #[test]
  fn local_client_server_round_trip_with_a_self_signed_certificate() {
    // Real, disclosed correction found by actually compiling this test
    // under plan 116's own `rcgen` 0.13 -> 0.14 bump (see that plan's
    // own Cargo.toml comment): `CertifiedKey`'s key-pair field was
    // renamed `key_pair` -> `signing_key` between those versions.
    let CertifiedKey { cert, signing_key } =
      generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
        .expect("self-signed test certificate generation should succeed");
    let cert_pem = cert.pem();
    let key_pem = signing_key.serialize_pem();

    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let cert_path = dir.join(format!("emerald_tls_test_cert_{pid}.pem"));
    let key_path = dir.join(format!("emerald_tls_test_key_{pid}.pem"));
    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, &key_pem).unwrap();

    let listener_id = unsafe {
      let host = c("127.0.0.1");
      let cert_path_c = c(cert_path.to_str().unwrap());
      let key_path_c = c(key_path.to_str().unwrap());
      tls_listen(host.as_ptr(), 0, cert_path_c.as_ptr(), key_path_c.as_ptr())
    };
    let port = handle_get_mut::<TlsListenerData, u16>(listener_id, TLS_LISTENER_TAG, |d| {
      d.listener.local_addr().unwrap().port()
    })
    .unwrap();

    let server_thread = std::thread::spawn(move || {
      let stream_id = tls_listener_accept(listener_id);
      let msg = unsafe { cstr(tls_stream_read(stream_id, 64)) };
      assert_eq!(msg, "ping");
      unsafe {
        let reply = c("pong");
        tls_stream_write(stream_id, reply.as_ptr());
      }
      tls_stream_close(stream_id);
    });

    let mut roots = RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let mut stream = connect_with_explicit_roots_for_test("127.0.0.1", port, roots)
      .expect("client-side handshake against the freshly generated cert should succeed");
    stream.write_all(b"ping").unwrap();
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"pong");

    server_thread.join().unwrap();
    std::fs::remove_file(&cert_path).ok();
    std::fs::remove_file(&key_path).ok();
  }
}
