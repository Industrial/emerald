//! Plan 105 (Multipart/Form-Data Parsing): mirrors `http_server.rs`'s
//! own dedicated-file convention (a `Http.serve`-based request/
//! response proof needing raw socket control — `Http.serve` never
//! returns on its own, the identical shape `http_server.rs`'s own
//! module doc already establishes for the same underlying reason).
//! Asserts on the HTTP response BODY, not stdout — see `examples/
//! multipart_upload_echo.em`'s own doc comment for the real,
//! disclosed finding this test's own design already accounts for:
//! `runtime/emerald_runtime.c`'s stdout-unbuffering trick is scoped
//! to `.register()`-ing actor processes only, so a `puts` inside an
//! ordinary `Http.serve` handler is not reliably visible in a piped
//! stdout before this test's own external `kill`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn compile_em(source: &str, tag: &str) -> PathBuf {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_multipart_{tag}_{}.em", std::process::id()));
  std::fs::write(&src_path, source).expect("should write source file");
  let out_path = dir.join(format!(
    "emerald_multipart_{tag}_bin_{}",
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
/// sleep — `http_server.rs`'s/`distributed_actors.rs`'s own
/// established idiom.
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

// A different, unlikely-to-collide fixed port from every other fixed
// port this session's own examples already spend (`http_server_
// proof.em`'s 47303, `websocket_proof.em`'s 47501, `sse_ticker.em`'s
// 47602) — matches `examples/multipart_upload_echo.em`'s own choice.
const PORT: u16 = 47701;

fn multipart_body(boundary: &str, field_name: &str, filename: &str, content: &[u8]) -> Vec<u8> {
  let mut body = Vec::new();
  body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
  body.extend_from_slice(
    format!("Content-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\n")
      .as_bytes(),
  );
  body.extend_from_slice(b"Content-Type: text/plain\r\n\r\n");
  body.extend_from_slice(content);
  body.extend_from_slice(b"\r\n");
  body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
  body
}

fn post_multipart(port: u16, path: &str, boundary: &str, body: &[u8]) -> (u16, String) {
  let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
  let head = format!(
    "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  );
  stream
    .write_all(head.as_bytes())
    .expect("write request head");
  stream.write_all(body).expect("write request body");
  let mut response = String::new();
  stream
    .read_to_string(&mut response)
    .expect("read HTTP response");
  let status_line = response.lines().next().unwrap_or("");
  let status: u16 = status_line
    .split_whitespace()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .unwrap_or(0);
  let resp_body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
  (status, resp_body)
}

// `leaf-example-and-gate`'s own worked-example proof: a real upload
// field, drained via repeated `Field.read_chunk` calls (65536-byte
// pieces per `examples/multipart_upload_echo.em`'s own text) rather
// than one whole-body read, echoing the field's own `name` and the
// exact total byte count back in the HTTP response body.
#[test]
fn multipart_upload_echo_em_prints_the_field_name_and_exact_byte_count() {
  let source = std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/multipart_upload_echo.em"),
  )
  .expect("should read examples/multipart_upload_echo.em");
  let bin = compile_em(&source, "upload_echo");

  let mut server = Command::new(&bin)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn multipart_upload_echo");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "server never started listening on port {PORT}"
  );

  let boundary = "EmeraldMultipartTestBoundary";
  // Deliberately larger than one 65536-byte `Field.read_chunk` call
  // (200_000 bytes) so the drain loop genuinely exercises more than
  // one real chunk, not just a single whole-body-sized read. `+ 1`
  // (range 1..=251, never 0) deliberately avoids an embedded NUL byte
  // — this crate's own established "`String` has no length header,
  // truncates at the first NUL" limit (`emerald-rt/src/lib.rs`'s own
  // module doc) would otherwise silently undercount whichever chunk
  // happened to contain one, corrupting this test's own exact-byte-
  // count assertion for a reason unrelated to what this test exists
  // to prove.
  let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8 + 1).collect();
  let body = multipart_body(boundary, "upload", "payload.bin", &content);
  let (status, resp_body) = post_multipart(PORT, "/upload", boundary, &body);
  assert_eq!(status, 200, "unexpected status; body was: {resp_body:?}");
  assert_eq!(
    resp_body,
    format!("upload\n{}", content.len()),
    "server's own response body should echo the field name then the exact byte count"
  );

  server.kill().expect("failed to kill server");
  server.wait().ok();
  std::fs::remove_file(&bin).ok();
}

// A request to any path other than `/upload` never even reaches the
// multipart machinery — `HttpResponse.build(404, "not found")`, the
// same shape `http_server_proof.em`'s own `/missing` case already
// establishes.
#[test]
fn a_request_to_an_unhandled_path_gets_a_plain_404_not_multipart_at_all() {
  let source = std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/multipart_upload_echo.em"),
  )
  .expect("should read examples/multipart_upload_echo.em");
  let bin = compile_em(&source, "upload_echo_404");

  let mut server = Command::new(&bin)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn multipart_upload_echo");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "server never started listening on port {PORT}"
  );

  let mut stream = TcpStream::connect(("127.0.0.1", PORT)).expect("connect");
  stream
    .write_all(b"GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .expect("write request");
  let mut response = String::new();
  stream
    .read_to_string(&mut response)
    .expect("read HTTP response");
  assert!(
    response.starts_with("HTTP/1.1 404"),
    "unexpected response: {response}"
  );
  assert!(
    response.ends_with("not found"),
    "unexpected response: {response}"
  );

  server.kill().expect("failed to kill server");
  server.wait().ok();
  std::fs::remove_file(&bin).ok();
}
