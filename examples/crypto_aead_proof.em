# Plan 110 (Symmetric AEAD Encryption) — `XChaCha20Poly1305.generate_
# key`/`.encrypt`/`.decrypt` (auto-nonce, `nonce || ciphertext || tag`
# packaging), proving: decrypting with the correct key recovers the
# exact original plaintext, and decrypting with a different key fails
# closed every time. `sealed`'s own bytes are NOT literally
# reproducible run-to-run (a fresh random nonce every `.encrypt` call,
# by design) — this proof checks the correctness PROPERTY, not a fixed
# ciphertext, exactly as this plan's own Decision log states.
#
# Real, disclosed deviations from the plan's own literal design:
# - `.unwrap()`/`.is_err()` (used in the plan's own Concrete Proof)
#   don't exist anywhere in this compiler — verified directly, no
#   match found in `emerald-sema`'s own source. `match X do Ok(v) do
#   ... end Err(e) do ... end end` is this compiler's real Result-
#   unwrapping idiom, used throughout.
# - `AeadError` is plain `String`, not a dedicated error type — the
#   same convention every other fallible native intrinsic already
#   uses (`Regex.compile`, every `humantime` parse function, ...).
# - The incremental key material must be freed explicitly via
#   `AeadKey#free()`, resolving this plan's own "Not yet decided item
#   1" EXECUTE blocker (real `zeroize`-on-drop, backed by plan 93's
#   handle registry) — not exercised for its own sake in this proof,
#   but present so a real program has a working way to do it.

key1: AeadKey = XChaCha20Poly1305.generate_key()
key2: AeadKey = XChaCha20Poly1305.generate_key()
plaintext: String = "attack at dawn"
aad: String = ""
plaintext_bytes: Bytes = plaintext.to_bytes()
aad_bytes: Bytes = aad.to_bytes()
plaintext_hex: String = plaintext_bytes.to_hex()

sealed_result: Result[Bytes, String] = XChaCha20Poly1305.encrypt(key1, plaintext_bytes, aad_bytes)
match sealed_result do
  Ok(sealed) do
    opened_result: Result[Bytes, String] = XChaCha20Poly1305.decrypt(key1, sealed, aad_bytes)
    match opened_result do
      Ok(opened) do
        opened_hex: String = opened.to_hex()
        matched: Boolean = opened_hex == plaintext_hex
        puts "#{matched}"
      end
      Err(msg) do
        puts msg
      end
    end

    wrong_key_result: Result[Bytes, String] = XChaCha20Poly1305.decrypt(key2, sealed, aad_bytes)
    match wrong_key_result do
      Ok(_leaked) do
        puts "false"
      end
      Err(_msg) do
        puts "true"
      end
    end
  end
  Err(msg) do
    puts msg
  end
end

key1.free()
key2.free()
