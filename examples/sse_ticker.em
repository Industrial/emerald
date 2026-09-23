# Plan 104 (Server-Sent Events) — a self-contained ticker: a single
# `/events` handler upgrades the request to an SSE stream, sends three
# `tick` events, then closes it. Like `websocket_proof.em`
# (plan 102) and `http_server_proof.em` (plan 101), `Http.serve` never
# returns, so unlike every other example in this repo's own
# `examples.rs` CI table this file's own process never exits on its
# own — `crates/emerald-cli/tests/examples.rs`'s own
# `sse_ticker_em_streams_three_framed_events_over_a_real_socket`
# spawns the compiled binary in the BACKGROUND, drives a real,
# plain `std::net::TcpStream` client directly from Rust against it,
# reads until the server closes the connection, and asserts on the
# exact byte sequence — the same "curl for the disclosed external
# check, a Rust test for the CI-enforced one" split this plan's own
# Decision log describes — and kills the process once the exchange
# completes.
#
# One real, disclosed deviation from this plan's own Concrete Proof
# text, found only by actually compiling this example against the
# real, current grammar (mirroring `http_server_proof.em`'s/
# `websocket_proof.em`'s own precedent, for the identical reason):
# `HttpServer.listen`/`.port`/`.accept` — the plan's own text names
# these "(assumed part of plan 101's own surface)" — do not actually
# exist; plan 101 only ever shipped `Http.serve(port) do |req| ... end`.
# This example upgrades the same accepted request `Http.serve`'s own
# trampoline already hands the handler, on a fixed port (matching
# `http_server_proof.em`/`websocket_proof.em`'s own precedent for the
# identical reason: no `HttpServer.port` readback exists to print an
# OS-chosen one), rather than the plan's own literal
# `HttpServer.listen(0)`/`.accept(server)` shape.

Http.serve(47602) do |req: HttpRequest|
  if req.path() == "/events" do
    stream: Int64 = Sse.upgrade(req)
    Sse.send(stream, "tick", "1")
    Sse.send(stream, "tick", "2")
    Sse.send(stream, "tick", "3")
    Sse.close(stream)
    return HttpResponse.build(200, "")
  end
  return HttpResponse.build(404, "not found")
end
