//! Plan 101 (HTTP Server): the plan's own explicitly self-contained
//! Concrete Proof — a real compiled `.em` server process, and a real
//! client (raw `TcpStream`, matching `emerald-rt`'s own unit-test
//! helper of the identical shape) both running entirely on
//! `127.0.0.1`, inside this CI sandbox, no external service required
//! (unlike plan 100's own `httpbin.org`-dependent proof). Mirrors
//! `distributed_actors.rs`'s own "compile, spawn in background, poll
//! the port, talk to it over a real socket, kill it" shape verbatim —
//! `Http.serve` never returns on its own (this plan's own Decision
//! log), the identical "own killing it after observing output" pattern
//! `distributed_actors.rs`'s own `host` process already establishes
//! for the same underlying reason (no remote-shutdown protocol).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn compile_em(source: &str, tag: &str) -> PathBuf {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!(
    "emerald_http_server_{tag}_{}.em",
    std::process::id()
  ));
  std::fs::write(&src_path, source).expect("should write source file");
  let out_path = dir.join(format!(
    "emerald_http_server_{tag}_bin_{}",
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .status()
    .expect("failed to run emerald-cli");
  assert!(status.success(), "emerald-cli should succeed on {tag}.em");
  std::fs::remove_file(&src_path).ok();
  out_path
}

/// Polls the port itself (a real TCP connect attempt), not a fixed
/// sleep — `distributed_actors.rs`'s own established idiom.
fn wait_for_port(port: u16, timeout: Duration) -> bool {
  let deadline = Instant::now() + timeout;
  while Instant::now() < deadline {
    if TcpStream::connect(("127.0.0.1", port)).is_ok() {
      return true;
    }
    std::thread::sleep(Duration::from_millis(20));
  }
  false
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

// A different, unlikely-to-collide fixed port from every other fixed
// port this session already spent (`raw_tcp_sockets.em`'s 47201,
// `dns_resolution.em`/`http_client_proof.em`'s network-dependent
// tests, `distributed_actors.rs`'s 19231/19232) — a deliberate choice,
// not a copy-paste.
const PORT: u16 = 47303;

#[test]
fn http_server_proof_em_hello_and_not_found_over_a_real_loopback_socket() {
  let source = std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/http_server_proof.em"),
  )
  .expect("should read examples/http_server_proof.em");
  let bin = compile_em(&source, "server_proof");

  let mut server = Command::new(&bin)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn http_server_proof");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "server never started listening on port {PORT}"
  );

  let (status, body) = raw_http_get(PORT, "/hello");
  assert_eq!(status, 200);
  assert_eq!(body, "hello GET");

  let (status, body) = raw_http_get(PORT, "/missing");
  assert_eq!(status, 404);
  assert_eq!(body, "not found");

  server.kill().expect("failed to kill server");
  server.wait().ok();
  std::fs::remove_file(&bin).ok();
}
