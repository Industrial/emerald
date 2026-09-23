# Plan 106 (Advanced Async HTTP, hyper-direct) — Concrete Proof:
# `Http2Client.get`/`.last_status`, wrapping `hyper`/`hyper-util`'s
# connection-pooling client directly, against `examples/http2_target_
# server.em` (started first, in the background, by `crates/emerald-
# cli/tests/http2_client.rs`'s own test). This `.em`-level output only
# checks the *values* are correct (any client could produce this
# output) — the plan's own actual claim, real TCP connection reuse
# across these three calls, is proven separately and deterministically
# by the Rust-side `#[test]` in `crates/emerald-rt/src/http2_client.rs`
# (`leaf-connection-reuse-proof`), which counts real accepted TCP
# connections rather than trusting this output text alone; see the
# plan's own Decision log for why the `.em`-level proof cannot observe
# pooling directly.

var i: Int64 = 0
while i < 3 do
  body: String = Http2Client.get("http://127.0.0.1:47801/")
  status: Int64 = Http2Client.last_status()
  puts status
  puts body
  i += 1
end
