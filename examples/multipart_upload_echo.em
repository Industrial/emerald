# Plan 105 (Multipart/Form-Data Parsing) — a self-contained upload
# echo: a single `/upload` handler parses a `multipart/form-data`
# request body, prints the first field's own `name`, then drains its
# content in bounded 65536-byte pieces via repeated `Field.read_chunk`
# calls (never materializing the whole upload as one `String`) and
# prints the exact total byte count. Like `http_server_proof.em`
# (plan 101), `Http.serve` never returns, so unlike every other example
# in this repo's own `examples.rs` CI table this file's own process
# never exits on its own — `crates/emerald-cli/tests/multipart.rs`'s
# own test spawns the compiled binary in the BACKGROUND, drives a real
# raw `multipart/form-data` POST directly over a `std::net::TcpStream`
# against it, and kills the process once the exchange completes.
#
# Two real, disclosed deviations from this plan's own Concrete Proof
# text, found only by actually compiling this example against the
# real, current grammar/type system (mirroring `sse_ticker.em`'s own
# precedent for the identical reason):
#
# 1. `HttpServer.listen`/`.port`/`.accept` — the plan's own text names
#    these "(assumed part of plan 101's own surface)" — do not exist;
#    plan 101 only ever shipped `Http.serve(port) do |req| ... end`.
#    This example upgrades the same accepted request `Http.serve`'s
#    own trampoline already hands the handler, on a fixed port
#    (`47701`, matching `http_server_proof.em`'s/`sse_ticker.em`'s own
#    precedent for the identical reason), rather than the plan's own
#    literal `HttpServer.listen(0)`/`.accept(server)` shape.
#    `HttpServer.content_type_boundary(req)` is similarly a genuine
#    `req.content_type_boundary()` instance method instead — see
#    `http_server.rs`'s own doc comment on that method.
# 2. `String?` is not this grammar's real nullable-type syntax — plan
#    73 replaced it with a genuine `Option[String]` sum type (`Some`/
#    `None`, matched via `match ... do Some(x) do ... end None do ...
#    end end`, confirmed directly against `examples/nullable_safe_
#    nav.em`'s own already-shipped, already-compiling text), predating
#    this plan's own authoring date. `while chunk != nil` is likewise
#    replaced with a `while true` loop that `break`s on `None`.
#
# A third, real, disclosed finding made only by actually running the
# compiled binary and killing it externally the same way `crates/
# emerald-cli/tests/multipart.rs`'s own CI-enforced test must (`Http.
# serve` never returns, on its own): `runtime/emerald_runtime.c`'s own
# `setvbuf(stdout, NULL, _IONBF, 0)` unbuffering trick is scoped
# EXCLUSIVELY to `.register()`-ing actor processes (its own comment
# says so directly) — an ordinary `Http.serve` process's stdout stays
# glibc's normal fully-block-buffered default when it isn't a TTY,
# so a `puts` inside a handler is genuinely NOT guaranteed visible in
# a piped/redirected stdout before an external kill (it would be, in
# a real interactive terminal, for this doc comment's own `curl`
# verification step below — the gap is CI-automation-specific). The
# `puts` calls below stay, matching this plan's own Concrete Proof
# text exactly, for that real interactive-terminal case; the response
# BODY additionally carries the same `name`/`total` info so `multipart.
# rs`'s own CI-enforced Rust test can assert on it reliably over the
# socket instead, the identical "curl for the disclosed external
# check, a Rust test for the CI-enforced one" split plan 104's own
# Decision log already establishes.

Http.serve(47701) do |req: HttpRequest|
  if req.path() == "/upload" do
    boundary: String = req.content_type_boundary()
    mp: Int64 = Multipart.start(req, boundary)
    field: Int64 = Multipart.next_field(mp)
    name: String = Field.name(field)
    puts name

    var total: Int64 = 0
    while true do
      chunk: Option[String] = Field.read_chunk(field, 65536)
      match chunk do
        Some(bytes) do
          total += bytes.length
        end
        None do
          break
        end
      end
    end
    puts total

    Field.close(field)
    return HttpResponse.build(200, "#{name}\n#{total}")
  end
  return HttpResponse.build(404, "not found")
end
