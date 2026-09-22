# Plan 111 (Asymmetric Cryptography and Digital Signatures) —
# `Ed25519.generate_key`/`.sign`/`.verify` and `X25519.
# generate_ephemeral`/`.diffie_hellman`, wrapping `ed25519-dalek`/
# `x25519-dalek`. `Rsa` is deliberately excluded from this example
# (this plan's own Decision log): `RsaPrivateKey::new`'s real,
# multi-second cost at 2048+ bits makes it unsuitable for a CI-run
# proof — it is instead covered by `emerald-rt`'s own round-trip unit
# test, verified separately from this compiled-and-run example.
#
# Key generation here is randomized by design — there is no fixed
# expected signature or shared-secret value this proof can assert.
# What's deterministic and actually checked: a signature verifies
# against the message it actually signed and the matching public key;
# the identical signature fails against a one-character-different
# message; and both parties in an X25519 exchange independently derive
# the exact same shared secret from their own private half and the
# other's public half — Diffie-Hellman's own defining correctness
# property.
#
# Real, disclosed deviations from the plan's own literal design:
# `.unwrap()`/`.is_ok()`/`.is_err()` (used in the plan's own Concrete
# Proof) don't exist anywhere in this compiler — the same finding
# plan 110's own example already made — `match X do Ok(v) do ... end
# Err(e) do ... end end` is used instead. `SignatureError` is plain
# `String`, the same convention every other fallible native intrinsic
# already uses. This grammar's chained-method-call limitation (no
# receiver may be a string literal or a prior method call's bare,
# non-block-attached result) applies here too — every intermediate
# value is bound to a local first.

signing_key: Ed25519KeyPair = Ed25519.generate_key()
verifying_key: Bytes = signing_key.public_key()
message: String = "attack at dawn"
message_bytes: Bytes = message.to_bytes()
signature: Bytes = signing_key.sign(message_bytes)

ok_result: Result[Void, String] = Ed25519.verify(verifying_key, message_bytes, signature)
match ok_result do
  Ok(_v) do
    puts "true"
  end
  Err(_msg) do
    puts "false"
  end
end

tampered: String = "attack at dusk"
tampered_bytes: Bytes = tampered.to_bytes()
bad_result: Result[Void, String] = Ed25519.verify(verifying_key, tampered_bytes, signature)
match bad_result do
  Ok(_v) do
    puts "false"
  end
  Err(_msg) do
    puts "true"
  end
end

alice: X25519EphemeralSecret = X25519.generate_ephemeral()
bob: X25519EphemeralSecret = X25519.generate_ephemeral()
alice_public: Bytes = alice.public_key()
bob_public: Bytes = bob.public_key()

alice_shared: Bytes = alice.diffie_hellman(bob_public)
bob_shared: Bytes = bob.diffie_hellman(alice_public)
alice_shared_hex: String = alice_shared.to_hex()
bob_shared_hex: String = bob_shared.to_hex()
matched: Boolean = alice_shared_hex == bob_shared_hex
puts "#{matched}"
