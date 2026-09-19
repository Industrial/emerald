//! Plan 60 (distributed, location-transparent actors),
//! `leaf-remote-dispatch-and-worked-proof` — the genuine two-process,
//! real-socket proof this plan's own claim needs: two SEPARATELY
//! compiled, separately launched OS processes, sharing one `actor`
//! declaration's source (spliced textually into both source strings
//! here rather than via a real multi-file `require`, a real, disclosed
//! simplification — both processes still agree on field layout/method
//! order/wire encoding exactly the same way plan 23's own `require`
//! splicing would guarantee, since the compiled text is byte-identical
//! either way), exchanging real messages over a real TCP socket.
//!
//! `host` never exits on its own once it calls `.register` (Design
//! decision 5 — no remote-shutdown protocol exists) — this test owns
//! killing it after observing its output, exactly as the plan's own
//! Decision log states.

use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn compile_em(source: &str, tag: &str) -> PathBuf {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!(
    "emerald_distributed_actors_{tag}_{}.em",
    std::process::id()
  ));
  std::fs::write(&src_path, source).expect("should write source file");
  let out_path = dir.join(format!(
    "emerald_distributed_actors_{tag}_bin_{}",
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

/// The plan's own worked example's shared `counter_actor.em`, verbatim.
const SHARED_ACTOR: &str = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn report: Void do\n    puts @count\n  end\nend\n\n";

fn host_src(port: u16) -> String {
  format!("{SHARED_ACTOR}c: Counter = Counter.spawn(0)\nc.register(\"counter1\", {port})\n")
}

fn client_src(port: u16) -> String {
  // Plan 60's Decision log: `remote` is a new reserved keyword (`.spawn`'s
  // own precedent — `spawn`/`new` are equally unusable as a plain local
  // name) — real, discovered writing this very test: `remote: Counter =
  // ...` doesn't parse. `handle`, not `remote`, is the local's name.
  format!(
    "{SHARED_ACTOR}handle: Counter = Counter.remote(\"127.0.0.1:{port}\", \"counter1\")\nhandle.increment\nhandle.increment\nhandle.increment\nhandle.report\n"
  )
}

/// Polls the port itself (a real TCP connect attempt), not a fixed
/// sleep — the plan's own stated requirement.
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

#[test]
fn counter_worked_example_produces_3_on_the_host_process_via_a_real_socket() {
  // A high, unusual port — real, disclosed collision risk if another
  // concurrently-running process on this machine happens to bind the
  // exact same port; not observed in practice for this port choice.
  const PORT: u16 = 19231;

  let host_bin = compile_em(&host_src(PORT), "dist_host");
  let client_bin = compile_em(&client_src(PORT), "dist_client");

  let mut host = Command::new(&host_bin)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn host");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "host never started listening on port {PORT}"
  );

  let client_status = Command::new(&client_bin)
    .status()
    .expect("failed to run client");
  assert!(
    client_status.success(),
    "client should exit successfully after its 3 increments + 1 report"
  );

  // `client`'s own process exit only proves its sends were WRITTEN to
  // the socket, not that `host`'s worker thread has finished PROCESSING
  // the last one yet — a short, bounded wait for that real async gap,
  // not a substitute for `wait_for_port`'s own real polling above.
  std::thread::sleep(Duration::from_millis(300));

  host.kill().expect("failed to kill host");
  host.wait().ok();
  let mut stdout = String::new();
  host
    .stdout
    .take()
    .expect("host's stdout was piped")
    .read_to_string(&mut stdout)
    .ok();

  assert!(
    stdout.contains('3'),
    "host's own stdout should show the final count 3 (mutated only by messages that arrived \
     over a real socket from the client's separate process/address space), got: {stdout:?}"
  );

  std::fs::remove_file(&host_bin).ok();
  std::fs::remove_file(&client_bin).ok();
}

/// AC4 (`leaf-remote-dispatch-and-worked-proof`): a process with at
/// least one `.register` call, given no incoming connections at all, is
/// confirmed still RUNNING (not exited) — proving the `drain_and_join`
/// extension itself, not merely assumed from the worked example above.
#[test]
fn a_registered_process_with_no_incoming_connections_stays_running() {
  const PORT: u16 = 19232;
  let host_bin = compile_em(&host_src(PORT), "dist_host_alone");

  let mut host = Command::new(&host_bin)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("failed to spawn host");

  assert!(
    wait_for_port(PORT, Duration::from_secs(5)),
    "host never started listening on port {PORT}"
  );
  // Give it a real window to have exited already, if it were going to.
  std::thread::sleep(Duration::from_millis(300));
  match host.try_wait() {
    Ok(None) => {} // still running — the expected outcome.
    Ok(Some(status)) => {
      panic!("host exited early with {status} — drain_and_join must block a `.register`ed process")
    }
    Err(e) => panic!("failed to poll host: {e}"),
  }

  host.kill().expect("failed to kill host");
  host.wait().ok();
  std::fs::remove_file(&host_bin).ok();
}
