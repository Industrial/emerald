# Plan 185 (OAuth2 Client Flow — Authorization Code Grant with
# Mandatory PKCE) — a self-contained proof of the real authorization-
# URL-building step (`OAuth2Client#begin_auth`, no network call) and
# the real network token-exchange round trip (`#exchange_code`),
# driven against a fixed local mock token endpoint
# `crates/emerald-cli/tests/examples.rs`'s own
# `oauth2_client_flow_em_exchanges_a_real_authorization_code_for_a_
# token` starts (a real `tiny_http` server, not a live third-party
# provider — the same "no live network dependency in CI" posture plan
# 96's own worked proof already established), listening on the exact
# fixed port (9931) this file's own literal `auth_url`/`token_url`
# name.
#
# Two real, disclosed deviations from this plan's own literal Concrete
# Proof text, found only by actually compiling this file against the
# real, current grammar — the same two `websocket_proof.em`/
# `extended_filesystem_proof.em` already disclose for the identical
# reason:
#
# (1) `case X when Pattern ... end` is not this grammar's real
# `Result`/enum-matching syntax — `match X do Pattern do ... end ...
# end` is (`examples/extended_filesystem_proof.em`'s own precedent).
#
# (2) `puts` accepts only `Int64`/`Float64`/`String` — `emerald-sema`'s
# own real `puts` intrinsic rejects `Boolean` with a compile
# diagnostic (`extended_filesystem_proof.em`'s own disclosed finding),
# reused here for `state.length > 0` — printed via string
# interpolation (`puts "#{state.length > 0}"`, `totp_2fa.em`'s own
# precedent) instead of a bare `puts state.length > 0`.
#
# A third, real addition beyond the plan's own literal proof text,
# disclosed rather than silently added: this file calls `Http.get`
# (plan 100) against the built `authorization_url` before exchanging
# the code — simulating the real browser step a genuine OAuth2 flow
# always has between "build the authorization URL" and "the provider
# redirects back with a code." Without this, the mock token endpoint
# would never actually observe the `code_challenge` this client
# generated, and could not perform the real, meaningful PKCE check
# plan 185's own Decision log describes ("the mock server itself can
# be configured to reject the exchange if the code_verifier it
# receives doesn't hash... to the code_challenge it saw in the
# authorization step") — this call is what lets the mock server
# actually see that step.

client: OAuth2Client = OAuth2Client.new(
  "test-client-id",
  "test-client-secret",
  "http://127.0.0.1:9931/authorize",
  "http://127.0.0.1:9931/token",
  "http://127.0.0.1:8080/callback"
)

scopes: Array[String] = ["profile", "email"]
auth_request: OAuth2AuthRequest = client.begin_auth(scopes, 2)

url: String = auth_request.authorization_url()
puts url

state: String = auth_request.state()
puts "#{state.length > 0}"

# The simulated browser step — see this file's own header comment.
Http.get(url)

exchange_result: Result[OAuth2Token, OAuth2Error] = client.exchange_code(auth_request, "fixed-test-code")
match exchange_result do
Ok(token) do
  puts token.access_token()
end
Err(e) do
  match e do
  TokenEndpoint(msg) do
    puts msg
  end
  Request(msg) do
    puts msg
  end
  Other(msg) do
    puts msg
  end
  end
end
end
