# Plan 102 (WebSocket) — a self-contained loopback echo test.
# `Http.serve` (plan 101) never returns, so unlike every other example
# in this repo's own `examples.rs` CI table this file's own process
# never exits on its own; `crates/emerald-cli/tests/examples.rs`'s own
# `websocket_proof_em_round_trips_a_real_echo_over_a_real_socket`
# spawns the compiled binary in the BACKGROUND, drives a real
# `tungstenite::connect("ws://127.0.0.1:47501/ws")` client directly
# from Rust against it (the reverse role from every other proof in
# this repo, since the *Emerald* side here is the server — this
# plan's own Concrete Proof's own framing), and kills the process once
# the exchange completes — never asserting against this file's own
# stdout at all.
#
# Two real, disclosed deviations from this plan's own Concrete Proof
# text, found only by actually compiling this example against the
# real, current grammar (mirroring `http_server_proof.em`'s own two
# disclosed corrections against plan 101's text, for the identical
# reason):
#
# (1) `case X when Pattern ... end` is not this grammar's real `Result`/
# enum-matching syntax — `match X do Pattern do ... end ... end` is
# (`examples/extended_filesystem_proof.em`'s own precedent, reused
# here).
#
# (2) Zero-argument instance methods (`req.path`, `msg.text`) are real
# calls and need real parens (`req.path()`, `msg.text()`) — confirmed
# directly against `examples/http_server_proof.em`'s own already-
# shipped, already-compiling text, not assumed from this plan's own
# paren-less pseudocode.

Http.serve(47501) do |req: HttpRequest|
  if req.path() == "/ws" do
    conn_result: Result[WebSocketConnection, WebSocketError] = req.upgrade()
    match conn_result do
    Ok(ws) do
      msg_result: Result[WebSocketMessage, WebSocketError] = ws.recv()
      match msg_result do
      Ok(msg) do
        ws.send_text("echo: " + msg.text())
      end
      Err(e) do
        puts "unexpected recv error"
      end
      end
      ws.close()
    end
    Err(e) do
      puts "unexpected upgrade error"
    end
    end
    return HttpResponse.build(200, "")
  end
  return HttpResponse.build(404, "not found")
end
