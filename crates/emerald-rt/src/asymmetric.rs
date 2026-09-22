//! Plan 111 (Asymmetric Cryptography and Digital Signatures) —
//! `Ed25519` (signing, `ed25519-dalek`), `X25519` (Diffie-Hellman key
//! exchange, `x25519-dalek`), and `Rsa` (PKCS#1 v1.5 encrypt/decrypt/
//! sign/verify, `rsa`) — three distinct problems, three distinct
//! crates, three distinct handle types, deliberately not conflated
//! (this plan's own Decision log).
//!
//! `Rsa` ships with a real, currently open, currently unpatched
//! RustSec advisory (RUSTSEC-2023-0071, the Marvin Attack — a timing
//! side-channel in private-key operations). `RsaKeyPair#decrypt`/
//! `#sign`'s own doc comments reproduce the advisory's stated
//! workaround verbatim: avoid using this in a setting where an
//! attacker can observe timing (e.g. over a network); local use on a
//! non-compromised machine is fine. `Rsa.encrypt`/`Rsa.verify`
//! (public-key-only operations) are not exposed to this attack class
//! at all.
//!
//! Real, disclosed simplifications vs. this plan's own literal design:
//! - `SignatureError`/`RsaError` are plain `String`, the same
//!   convention every other fallible native intrinsic in this crate
//!   already uses (plans 109/110's own identical choice).
//! - `Rsa.encrypt`/`.decrypt`/`.sign`/`Rsa.verify` all take the single
//!   combined `RsaKeyPair` handle (never a separate `RsaPublicKey`
//!   type this plan's own todo text names but its own Decision log
//!   explicitly declines to split out) — `RsaPublicKey::from(&priv)`
//!   is a cheap, direct derivation, not new cryptographic logic.
//! - Key/seed generation for `Ed25519`/`X25519` goes through
//!   `getrandom::fill` (`Ed25519`) or each crate's own no-argument
//!   `::random()` constructor (`X25519`, gated by its own `getrandom`
//!   feature) — `SigningKey::generate`'s own RNG-trait argument is
//!   deliberately not used, avoiding a second RNG-bridging convention
//!   in a crate that already has one working path (`getrandom::fill`,
//!   plan 110's own precedent). `Rsa` needs a real `rand_core 0.6`-
//!   generation `CryptoRng` (the crate's own dependency is pinned one
//!   generation behind the `getrandom`/`rand_core 0.10` ecosystem
//!   everything else here uses) — `rand` 0.8's `OsRng` is added
//!   specifically for this, a real, disclosed extra dependency this
//!   plan's own text didn't anticipate.

use crate::bytes::{bytes_as_slice, bytes_from_slice};
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rsa::{Pkcs1v15Encrypt, Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey, StaticSecret};

const ED25519_TAG: &str = "Ed25519KeyPair";
const X25519_EPHEMERAL_TAG: &str = "X25519EphemeralSecret";
const X25519_STATIC_TAG: &str = "X25519StaticSecret";
const RSA_TAG: &str = "RsaKeyPair";

// ---- Ed25519 --------------------------------------------------------

/// `Ed25519.generate_key(): Ed25519KeyPair`.
pub fn ed25519_generate_key() -> i64 {
  let mut seed = [0u8; 32];
  getrandom::fill(&mut seed).expect("emerald-rt: OS CSPRNG failure generating an Ed25519 seed");
  handle_alloc(Box::new(SigningKey::from_bytes(&seed)), ED25519_TAG)
}

/// `Ed25519KeyPair#sign(self, msg: Bytes): Bytes` — a 64-byte
/// signature.
///
/// # Safety
/// `msg_id` must be a live `Bytes` value.
pub unsafe fn ed25519_sign(id: i64, msg_id: i64) -> i64 {
  let msg = bytes_as_slice(msg_id);
  match handle_get_mut::<SigningKey, Vec<u8>>(id, ED25519_TAG, |sk| {
    sk.sign(msg).to_bytes().to_vec()
  }) {
    Ok(sig) => bytes_from_slice(&sig),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Ed25519KeyPair#public_key(self): Bytes` — the 32-byte verifying
/// key.
pub fn ed25519_public_key(id: i64) -> i64 {
  match handle_get_mut::<SigningKey, Vec<u8>>(id, ED25519_TAG, |sk| {
    sk.verifying_key().to_bytes().to_vec()
  }) {
    Ok(pk) => unsafe { bytes_from_slice(&pk) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Ed25519.verify(pubkey: Bytes, msg: Bytes, sig: Bytes): Result[Void,
/// String]` — `verify_strict`, not plain `verify` (this plan's own
/// Decision log: rules out the "weak key forgery" class plain
/// `verify` permits).
///
/// # Safety
/// `pubkey_id`/`msg_id`/`sig_id` must be live `Bytes` values.
pub unsafe fn ed25519_verify(pubkey_id: i64, msg_id: i64, sig_id: i64) -> *mut std::ffi::c_void {
  let pubkey = bytes_as_slice(pubkey_id);
  let msg = bytes_as_slice(msg_id);
  let sig = bytes_as_slice(sig_id);
  let pubkey: [u8; 32] = match pubkey.try_into() {
    Ok(p) => p,
    Err(_) => return crate::emerald_rt_result_err_str("Ed25519 public key must be 32 bytes"),
  };
  let sig: [u8; 64] = match sig.try_into() {
    Ok(s) => s,
    Err(_) => return crate::emerald_rt_result_err_str("Ed25519 signature must be 64 bytes"),
  };
  let vk = match VerifyingKey::from_bytes(&pubkey) {
    Ok(vk) => vk,
    Err(e) => return crate::emerald_rt_result_err_str(&e.to_string()),
  };
  let signature = Signature::from_bytes(&sig);
  match vk.verify_strict(msg, &signature) {
    Ok(()) => crate::emerald_rt_result_ok(0),
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

// ---- X25519 ----------------------------------------------------------

/// `X25519.generate_ephemeral(): X25519EphemeralSecret`.
pub fn x25519_generate_ephemeral() -> i64 {
  handle_alloc(Box::new(EphemeralSecret::random()), X25519_EPHEMERAL_TAG)
}

/// `X25519.generate_static(): X25519StaticSecret`.
pub fn x25519_generate_static() -> i64 {
  handle_alloc(Box::new(StaticSecret::random()), X25519_STATIC_TAG)
}

/// `X25519EphemeralSecret#public_key(self): Bytes`.
pub fn x25519_ephemeral_public_key(id: i64) -> i64 {
  match handle_get_mut::<EphemeralSecret, Vec<u8>>(id, X25519_EPHEMERAL_TAG, |sk| {
    X25519PublicKey::from(&*sk).to_bytes().to_vec()
  }) {
    Ok(pk) => unsafe { bytes_from_slice(&pk) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `X25519StaticSecret#public_key(self): Bytes`.
pub fn x25519_static_public_key(id: i64) -> i64 {
  match handle_get_mut::<StaticSecret, Vec<u8>>(id, X25519_STATIC_TAG, |sk| {
    X25519PublicKey::from(&*sk).to_bytes().to_vec()
  }) {
    Ok(pk) => unsafe { bytes_from_slice(&pk) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

fn parse_x25519_public(bytes: &[u8]) -> Result<X25519PublicKey, String> {
  let arr: [u8; 32] = bytes
    .try_into()
    .map_err(|_| "X25519 public key must be 32 bytes".to_string())?;
  Ok(X25519PublicKey::from(arr))
}

/// `X25519EphemeralSecret#diffie_hellman(self, their_public: Bytes):
/// Bytes` — consumes the handle (per the underlying Rust type's own
/// single-use design; see this plan's own Decision log on the real,
/// disclosed gap this can't fully restore across the FFI boundary).
///
/// # Safety
/// `their_public_id` must be a live `Bytes` value.
pub unsafe fn x25519_ephemeral_diffie_hellman(id: i64, their_public_id: i64) -> i64 {
  let their_public = match parse_x25519_public(bytes_as_slice(their_public_id)) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<EphemeralSecret, Vec<u8>>(id, X25519_EPHEMERAL_TAG, |sk| {
    let taken = std::mem::replace(sk, EphemeralSecret::random());
    taken.diffie_hellman(&their_public).to_bytes().to_vec()
  });
  match result {
    Ok(shared) => {
      handle_close(id);
      bytes_from_slice(&shared)
    }
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `X25519StaticSecret#diffie_hellman(self, their_public: Bytes):
/// Bytes` — reusable, does not consume the handle.
///
/// # Safety
/// `their_public_id` must be a live `Bytes` value.
pub unsafe fn x25519_static_diffie_hellman(id: i64, their_public_id: i64) -> i64 {
  let their_public = match parse_x25519_public(bytes_as_slice(their_public_id)) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<StaticSecret, Vec<u8>>(id, X25519_STATIC_TAG, |sk| {
    sk.diffie_hellman(&their_public).to_bytes().to_vec()
  }) {
    Ok(shared) => bytes_from_slice(&shared),
    Err(e) => crate::raise_native_error(&e),
  }
}

// ---- RSA ---------------------------------------------------------------

/// `Rsa.generate_key(bits: Int64): Result[RsaKeyPair, String]` — real,
/// multi-second cost at 2048+ bits, documented not hidden (this plan's
/// own Decision log).
pub fn rsa_generate_key(bits: i64) -> *mut std::ffi::c_void {
  if bits <= 0 {
    return unsafe { crate::emerald_rt_result_err_str("Rsa.generate_key: bits must be positive") };
  }
  let mut rng = rand::rngs::OsRng;
  match RsaPrivateKey::new(&mut rng, bits as usize) {
    Ok(key) => {
      let id = handle_alloc(Box::new(key), RSA_TAG);
      unsafe { crate::emerald_rt_result_ok(id) }
    }
    Err(e) => unsafe { crate::emerald_rt_result_err_str(&e.to_string()) },
  }
}

/// `Rsa.encrypt(keypair: RsaKeyPair, data: Bytes): Result[Bytes,
/// String]` — public-key operation, not affected by RUSTSEC-2023-0071.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn rsa_encrypt(keypair_id: i64, data_id: i64) -> *mut std::ffi::c_void {
  let data = bytes_as_slice(data_id);
  let mut rng = rand::rngs::OsRng;
  let result =
    handle_get_mut::<RsaPrivateKey, Result<Vec<u8>, String>>(keypair_id, RSA_TAG, |priv_key| {
      let pub_key = RsaPublicKey::from(&*priv_key);
      pub_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, data)
        .map_err(|e| e.to_string())
    });
  match result {
    Ok(Ok(ciphertext)) => crate::emerald_rt_result_ok(bytes_from_slice(&ciphertext)),
    Ok(Err(e)) => crate::emerald_rt_result_err_str(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `RsaKeyPair#decrypt(self, data: Bytes): Result[Bytes, String]` —
/// private-key operation. RUSTSEC-2023-0071 (Marvin Attack): avoid
/// using this in a setting where an attacker can observe timing (e.g.
/// over a network); local use on a non-compromised machine is fine —
/// quoted from the advisory's own stated workaround.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn rsa_decrypt(keypair_id: i64, data_id: i64) -> *mut std::ffi::c_void {
  let data = bytes_as_slice(data_id);
  let result =
    handle_get_mut::<RsaPrivateKey, Result<Vec<u8>, String>>(keypair_id, RSA_TAG, |priv_key| {
      priv_key
        .decrypt(Pkcs1v15Encrypt, data)
        .map_err(|e| e.to_string())
    });
  match result {
    Ok(Ok(plaintext)) => crate::emerald_rt_result_ok(bytes_from_slice(&plaintext)),
    Ok(Err(e)) => crate::emerald_rt_result_err_str(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `RsaKeyPair#sign(self, digest: Bytes): Result[Bytes, String]` —
/// private-key operation; the same RUSTSEC-2023-0071 workaround as
/// `#decrypt` applies. `digest` is expected to be the output of
/// `Sha256.hash` (plan 109).
///
/// # Safety
/// `digest_id` must be a live `Bytes` value.
pub unsafe fn rsa_sign(keypair_id: i64, digest_id: i64) -> *mut std::ffi::c_void {
  let digest = bytes_as_slice(digest_id);
  let result =
    handle_get_mut::<RsaPrivateKey, Result<Vec<u8>, String>>(keypair_id, RSA_TAG, |priv_key| {
      // `new_unprefixed()`, not `new::<Sha256>()` — real, disclosed
      // simplification found compiling this: the OID-typed
      // constructor requires the digest type to implement `rsa::
      // pkcs8::AssociatedOid` against a `const-oid` major that this
      // crate's own dependency graph resolves inconsistently across
      // `sha2` 0.11 (plan 109) and even a separately-pinned `sha2`
      // 0.10 (both tried; both hit the identical trait-bound
      // mismatch). `new_unprefixed()` signs the bare 32-byte digest
      // with no ASN.1 DigestInfo prefix — standards-compliant PKCS#1
      // v1.5 signing normally includes that prefix to identify the
      // hash algorithm to an independent verifier; this plan's own
      // `Rsa.verify` uses the identical unprefixed scheme, so
      // round-tripping through this module's own wrapper is fully
      // correct, but a signature produced here is not directly
      // interoperable with an external strict-PKCS1v15 verifier
      // expecting the standard prefixed form.
      priv_key
        .sign(Pkcs1v15Sign::new_unprefixed(), digest)
        .map_err(|e| e.to_string())
    });
  match result {
    Ok(Ok(sig)) => crate::emerald_rt_result_ok(bytes_from_slice(&sig)),
    Ok(Err(e)) => crate::emerald_rt_result_err_str(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Rsa.verify(keypair: RsaKeyPair, digest: Bytes, sig: Bytes):
/// Result[Void, String]` — public-key operation, not affected by
/// RUSTSEC-2023-0071.
///
/// # Safety
/// `digest_id`/`sig_id` must be live `Bytes` values.
pub unsafe fn rsa_verify(keypair_id: i64, digest_id: i64, sig_id: i64) -> *mut std::ffi::c_void {
  let digest = bytes_as_slice(digest_id);
  let sig = bytes_as_slice(sig_id);
  let result =
    handle_get_mut::<RsaPrivateKey, Result<(), String>>(keypair_id, RSA_TAG, |priv_key| {
      let pub_key = RsaPublicKey::from(&*priv_key);
      // `new_unprefixed()` — see `rsa_sign`'s own comment for why.
      pub_key
        .verify(Pkcs1v15Sign::new_unprefixed(), digest, sig)
        .map_err(|e| e.to_string())
    });
  match result {
    Ok(Ok(())) => crate::emerald_rt_result_ok(0),
    Ok(Err(e)) => crate::emerald_rt_result_err_str(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  unsafe fn as_bytes(s: &[u8]) -> i64 {
    bytes_from_slice(s)
  }

  fn ok_id(result_ptr: *mut std::ffi::c_void) -> i64 {
    unsafe {
      let ptr = result_ptr as *const i64;
      assert_eq!(*ptr, 0, "expected Ok");
      *ptr.add(1)
    }
  }

  fn is_err(result_ptr: *mut std::ffi::c_void) -> bool {
    unsafe { *(result_ptr as *const i64) == 1 }
  }

  // RFC 8032 SS7.1, TEST 1: empty message, fixed key material.
  #[test]
  fn ed25519_matches_the_rfc_8032_test_1_vector() {
    let seed_bytes =
      hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").unwrap();
    let seed: [u8; 32] = seed_bytes.try_into().unwrap();
    let sk = SigningKey::from_bytes(&seed);
    let expected_pk =
      hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a").unwrap();
    assert_eq!(sk.verifying_key().to_bytes().to_vec(), expected_pk);
    let sig = sk.sign(&[]);
    let expected_sig = hex::decode(concat!(
      "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
      "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    ))
    .unwrap();
    assert_eq!(sig.to_bytes().to_vec(), expected_sig);
  }

  #[test]
  fn ed25519_sign_and_verify_round_trip_and_a_tampered_message_fails() {
    unsafe {
      let kp = ed25519_generate_key();
      let msg = as_bytes(b"attack at dawn");
      let sig = ed25519_sign(kp, msg);
      let pk = ed25519_public_key(kp);
      let ok = ed25519_verify(pk, msg, sig);
      assert!(!is_err(ok), "correct signature should verify");

      let tampered = as_bytes(b"attack at dusk");
      let bad = ed25519_verify(pk, tampered, sig);
      assert!(is_err(bad), "tampered message must fail verification");
    }
  }

  // RFC 7748 SS6.1: Alice/Bob's fixed X25519 private scalars and the
  // resulting shared secret.
  #[test]
  fn x25519_matches_the_rfc_7748_alice_bob_test_vector() {
    let alice_private =
      hex::decode("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a").unwrap();
    let bob_private =
      hex::decode("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb").unwrap();
    let expected_shared =
      hex::decode("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742").unwrap();

    let alice_scalar: [u8; 32] = alice_private.try_into().unwrap();
    let bob_scalar: [u8; 32] = bob_private.try_into().unwrap();
    let alice_secret = StaticSecret::from(alice_scalar);
    let bob_secret = StaticSecret::from(bob_scalar);
    let alice_public = X25519PublicKey::from(&alice_secret);
    let bob_public = X25519PublicKey::from(&bob_secret);

    let alice_shared = alice_secret.diffie_hellman(&bob_public);
    let bob_shared = bob_secret.diffie_hellman(&alice_public);
    assert_eq!(alice_shared.to_bytes().to_vec(), expected_shared);
    assert_eq!(bob_shared.to_bytes().to_vec(), expected_shared);
  }

  #[test]
  fn x25519_ephemeral_exchange_agrees_both_directions() {
    unsafe {
      let alice = x25519_generate_ephemeral();
      let bob = x25519_generate_ephemeral();
      let alice_public = x25519_ephemeral_public_key(alice);
      let bob_public = x25519_ephemeral_public_key(bob);

      let alice_shared_id = x25519_ephemeral_diffie_hellman(alice, bob_public);
      let bob_shared_id = x25519_ephemeral_diffie_hellman(bob, alice_public);
      assert_eq!(
        bytes_as_slice(alice_shared_id),
        bytes_as_slice(bob_shared_id)
      );
    }
  }

  #[test]
  fn rsa_round_trips_encrypt_decrypt_and_sign_verify() {
    unsafe {
      let kp_result = rsa_generate_key(2048);
      let kp = ok_id(kp_result);

      let data = as_bytes(b"hello rsa");
      let ct_result = rsa_encrypt(kp, data);
      let ct = ok_id(ct_result);
      let pt_result = rsa_decrypt(kp, ct);
      let pt = ok_id(pt_result);
      assert_eq!(bytes_as_slice(pt), b"hello rsa");

      let digest = as_bytes(&[0x11; 32]);
      let sig_result = rsa_sign(kp, digest);
      let sig = ok_id(sig_result);
      let verify_result = rsa_verify(kp, digest, sig);
      assert!(!is_err(verify_result), "correct signature should verify");
    }
  }
}
