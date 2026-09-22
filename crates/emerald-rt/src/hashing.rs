//! Plan 109 (Cryptographic Hashing) — `Sha256`/`Sha512` (`sha2`),
//! `Sha3_256`/`Sha3_512` (`sha3`), `Blake3` (`blake3`), `Md5` (`md-5`,
//! legacy-interop-only — see `md5_hash`'s own doc comment) one-shot
//! digests, plus `Sha256Hasher`/`Blake3Hasher` incremental handles
//! backed by plan 93's own `crate::handle` registry — the two
//! structurally distinct streaming shapes (Merkle-Damgård vs.
//! Merkle-tree) this plan's Decision log chose to prove once each,
//! rather than wiring all six algorithms for incremental use. Every
//! function takes/returns a `Bytes` value via `bytes.rs`'s own
//! `bytes_as_slice`/`bytes_from_slice` — see that module's own doc
//! comment for `Bytes`'s real, disclosed representation.
//!
//! This plan invents no new cryptographic construction — every
//! function here is a direct, unmodified call into its crate's own
//! `Digest`/`hash` entry point (plan 109's own Decision log).

use crate::bytes::{bytes_as_slice, bytes_from_slice};
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use md5::{Digest as _, Md5};
use sha2::{Sha256, Sha512};
use sha3::{Sha3_256, Sha3_512};

const SHA256_HASHER_TAG: &str = "Sha256Hasher";
const BLAKE3_HASHER_TAG: &str = "Blake3Hasher";

/// `Sha256.hash(bytes: Bytes): Bytes`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn sha256_hash(bytes_id: i64) -> i64 {
  let digest = Sha256::digest(bytes_as_slice(bytes_id));
  bytes_from_slice(&digest)
}

/// `Sha512.hash(bytes: Bytes): Bytes`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn sha512_hash(bytes_id: i64) -> i64 {
  let digest = Sha512::digest(bytes_as_slice(bytes_id));
  bytes_from_slice(&digest)
}

/// `Sha3_256.hash(bytes: Bytes): Bytes`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn sha3_256_hash(bytes_id: i64) -> i64 {
  let digest = Sha3_256::digest(bytes_as_slice(bytes_id));
  bytes_from_slice(&digest)
}

/// `Sha3_512.hash(bytes: Bytes): Bytes`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn sha3_512_hash(bytes_id: i64) -> i64 {
  let digest = Sha3_512::digest(bytes_as_slice(bytes_id));
  bytes_from_slice(&digest)
}

/// `Blake3.hash(bytes: Bytes): Bytes`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn blake3_hash(bytes_id: i64) -> i64 {
  let hash = blake3::hash(bytes_as_slice(bytes_id));
  bytes_from_slice(hash.as_bytes())
}

/// `Md5.hash(bytes: Bytes): Bytes` — quoting RustCrypto's own README
/// verbatim, not a paraphrase: "MD5 is cryptographically broken and
/// unsuitable for further use... provided for the purposes of legacy
/// interoperability with protocols and systems which mandate the use
/// of MD5" only. Never reach for this for anything security-sensitive.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn md5_hash(bytes_id: i64) -> i64 {
  let digest = Md5::digest(bytes_as_slice(bytes_id));
  bytes_from_slice(&digest)
}

/// `Sha256.new(): Sha256Hasher`.
pub fn sha256_hasher_new() -> i64 {
  handle_alloc(Box::new(Sha256::new()), SHA256_HASHER_TAG)
}

/// `Sha256Hasher#update(self, data: Bytes): Void`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn sha256_hasher_update(id: i64, bytes_id: i64) {
  let data = bytes_as_slice(bytes_id);
  if let Err(msg) = handle_get_mut::<Sha256, ()>(id, SHA256_HASHER_TAG, |h| h.update(data)) {
    crate::raise_native_error(&msg);
  }
}

/// `Sha256Hasher#finalize(self): Bytes` — consumes the handle (frees
/// the boxed `Sha256` state via `handle_close`, per plan 93's own
/// registry mechanism this plan defers its handle's lifetime story
/// to).
pub fn sha256_hasher_finalize(id: i64) -> i64 {
  match handle_get_mut::<Sha256, Vec<u8>>(id, SHA256_HASHER_TAG, |h| {
    let taken = std::mem::replace(h, Sha256::new());
    taken.finalize().to_vec()
  }) {
    Ok(digest) => {
      handle_close(id);
      unsafe { bytes_from_slice(&digest) }
    }
    Err(msg) => unsafe { crate::raise_native_error(&msg) },
  }
}

/// `Blake3.new(): Blake3Hasher`.
pub fn blake3_hasher_new() -> i64 {
  handle_alloc(Box::new(blake3::Hasher::new()), BLAKE3_HASHER_TAG)
}

/// `Blake3Hasher#update(self, data: Bytes): Void`.
///
/// # Safety
/// `bytes_id` must be a live `Bytes` value.
pub unsafe fn blake3_hasher_update(id: i64, bytes_id: i64) {
  let data = bytes_as_slice(bytes_id);
  if let Err(msg) = handle_get_mut::<blake3::Hasher, ()>(id, BLAKE3_HASHER_TAG, |h| {
    h.update(data);
  }) {
    crate::raise_native_error(&msg);
  }
}

/// `Blake3Hasher#finalize(self): Bytes` — consumes the handle the same
/// way `Sha256Hasher#finalize` does, even though `blake3::Hasher::
/// finalize` itself takes `&self` (a Merkle-tree hash can finalize
/// without moving its own state, unlike SHA-256's Merkle-Damgård
/// `Digest::finalize(self)`) — `handle_close` still runs afterward so
/// both hashers share the identical "finalize consumes" contract at
/// the Emerald-source level.
pub fn blake3_hasher_finalize(id: i64) -> i64 {
  match handle_get_mut::<blake3::Hasher, [u8; 32]>(id, BLAKE3_HASHER_TAG, |h| {
    *h.finalize().as_bytes()
  }) {
    Ok(digest) => {
      handle_close(id);
      unsafe { bytes_from_slice(&digest) }
    }
    Err(msg) => unsafe { crate::raise_native_error(&msg) },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::bytes::{bytes_as_slice, bytes_from_slice};

  unsafe fn bytes_of(s: &str) -> i64 {
    bytes_from_slice(s.as_bytes())
  }

  // SHA-256("hello world") — sha2's own crate-level README example.
  #[test]
  fn sha256_hash_matches_the_published_test_vector() {
    unsafe {
      let id = sha256_hash(bytes_of("hello world"));
      assert_eq!(
        hex::encode(bytes_as_slice(id)),
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
      );
    }
  }

  // SHA3-256("abc") — sha3's own crate-level README example.
  #[test]
  fn sha3_256_hash_matches_the_published_test_vector() {
    unsafe {
      let id = sha3_256_hash(bytes_of("abc"));
      assert_eq!(
        hex::encode(bytes_as_slice(id)),
        "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
      );
    }
  }

  // MD5("hello world") — md-5's own crate-level README example.
  #[test]
  fn md5_hash_matches_the_published_test_vector() {
    unsafe {
      let id = md5_hash(bytes_of("hello world"));
      assert_eq!(
        hex::encode(bytes_as_slice(id)),
        "5eb63bbbe01eeed093cb22bb8f5acdc3"
      );
    }
  }

  // BLAKE3("") — the `input_len: 0` case's first 32 output bytes from
  // the BLAKE3 project's own canonical test_vectors.json.
  #[test]
  fn blake3_hash_of_empty_input_matches_the_published_test_vector() {
    unsafe {
      let id = blake3_hash(bytes_of(""));
      assert_eq!(
        hex::encode(bytes_as_slice(id)),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
      );
    }
  }

  #[test]
  fn incremental_sha256_agrees_with_the_one_shot_digest() {
    unsafe {
      let one_shot = sha256_hash(bytes_of("hello world"));
      let hasher = sha256_hasher_new();
      sha256_hasher_update(hasher, bytes_of("hello "));
      sha256_hasher_update(hasher, bytes_of("world"));
      let streamed = sha256_hasher_finalize(hasher);
      assert_eq!(bytes_as_slice(one_shot), bytes_as_slice(streamed));
    }
  }

  #[test]
  fn incremental_blake3_agrees_with_the_one_shot_digest() {
    unsafe {
      let one_shot = blake3_hash(bytes_of("hello world"));
      let hasher = blake3_hasher_new();
      blake3_hasher_update(hasher, bytes_of("hello "));
      blake3_hasher_update(hasher, bytes_of("world"));
      let streamed = blake3_hasher_finalize(hasher);
      assert_eq!(bytes_as_slice(one_shot), bytes_as_slice(streamed));
    }
  }

  // A closed-handle reuse (`sha256_hasher_update` after `.finalize()`)
  // is deliberately NOT exercised by any in-process test here — see
  // `test_stubs`'s own doc comment in `lib.rs`: this crate's `raise_
  // native_error` ultimately calls the real `emerald_raise` C export,
  // stubbed in test builds to `std::process::abort()` (a real panic
  // escaping a plain `extern "C" fn` boundary is genuine UB Rust's own
  // runtime refuses to allow), so `catch_unwind` around it aborts the
  // whole test process rather than catching anything. The real,
  // end-to-end behavior (a closed handle raising a real, rescuable
  // `NativeError`) is verified the same way plan 92's own panic-path
  // proof is: through a real `.em` example run via the real CLI, not
  // an in-process unit test.
}
