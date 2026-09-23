# Plan 106 (Advanced Async HTTP, hyper-direct) — companion server for
# `examples/http2_client.em`'s own Concrete Proof: a plain plan-101
# `tiny_http`-backed `Http.serve` responding `200 OK` with a small,
# fixed body to every request. Deliberately HTTP/1.1-only (`tiny_http`
# itself has no HTTP/2 support at all — a real, disclosed limitation
# of using it as this plan's own local proof target, named in the
# plan's own Decision log): `hyper`'s client correctly negotiates down
# to HTTP/1.1 against it, which is sufficient to prove connection
# *reuse* (this plan's actual stated benefit) without exercising `h2`'s
# own wire format. Like `http_server_proof.em`/`multipart_upload_echo.
# em`, `Http.serve` never returns, so this process is spawned in the
# BACKGROUND by `crates/emerald-cli/tests/http2_client.rs`'s own test
# and killed once the exchange completes.

Http.serve(47801) do |req: HttpRequest|
  return HttpResponse.build(200, "http2-target-ok")
end
