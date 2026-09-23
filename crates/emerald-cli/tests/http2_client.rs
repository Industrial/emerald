//! Plan 106 (Advanced Async HTTP, hyper-direct): mirrors `multipart.
//! rs`'s own dedicated-file convention (a `Http.serve`-based proof
//! needing a real background server process — `Http.serve` never
//! returns on its own). Compiles and spawns `examples/http2_target_
//! server.em` in the background, then compiles and runs `examples/
//! http2_client.em` to completion, asserting on its real captured
//! stdout — the client here is an ordinary, non-`Http.serve` process
//! (it exits on its own once its `while` loop finishes), so unlike
//! `http_server_proof.em`'s/`multipart_upload_echo.em`'s own handler-
//! side `puts` calls, this one's stdout is NOT subject to `runtime/
//! emerald_runtime.c`'s own `Http.serve`-scoped unbuffering gap —
//! asserting on stdout here is the real, direct Concrete Proof check,
//! not a workaround.

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn compile_em(source: &str, tag: &str) -> PathBuf {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_http2_{tag}_{}.em", std::process::id()));
  std::fs::write(&src_path, source).expect("should write source file");
  let out_path = dir.join(format!("emerald_http2_{tag}_bin_{}", std::process::id()));

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
/// sleep — `http_server.rs`'s/`multipart.rs`'s own established idiom.
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
// 47602, `multipart_upload_echo.em`'s 47701) — matches `examples/
// http2_target_server.em`'s own choice.
const PORT: u16 = 47801;

fn read_example(name: &str) -> String {
  std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../examples/{name}")),
  )
  .unwrap_or_else(|e| panic!("should read examples/{name}: {e}"))
}

// `leaf-example-and-gate`'s own worked-example proof: three sequential
// `Http2Client.get` calls against the companion `Http.serve` target,
// each printing its real status then its real body — the plan's own
// Concrete Proof text verbatim. The separate, deterministic proof
// that these three calls actually reused one pooled TCP connection
// (this plan's real stated benefit) lives in `crates/emerald-rt/src/
// http2_client.rs`'s own `#[test]` (`leaf-connection-reuse-proof`),
// per the plan's own Decision log on why `.em`-level output alone
// cannot distinguish pooled from unpooled requests.
#[test]
fn http2_client_get_against_the_target_server_prints_status_and_body_three_times() {
  let server_source = read_example("http2_target_server.em");
  let server_bin = compile_em(&server_source, "target_server");

  let mut server = Command::new(&server_bin)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn http2_target_server");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "server never started listening on port {PORT}"
  );

  let client_source = read_example("http2_client.em");
  let client_bin = compile_em(&client_source, "client");

  let output = Command::new(&client_bin)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .expect("failed to run http2_client");

  server.kill().expect("failed to kill server");
  server.wait().ok();
  std::fs::remove_file(&server_bin).ok();
  std::fs::remove_file(&client_bin).ok();

  assert!(
    output.status.success(),
    "http2_client should exit successfully; stderr was: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  let expected = "200\nhttp2-target-ok\n".repeat(3);
  assert_eq!(
    stdout,
    expected,
    "expected three status/body pairs; stderr was: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}
