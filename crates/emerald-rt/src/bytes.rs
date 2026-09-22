//! Plan 109's Decision log: `Bytes` — NOT a new `Type` variant and NOT
//! a genuine two-word `{ptr, len}` fat value (the plan's own literal,
//! provisional shape, left open by its own "Not yet decided" item 1).
//! Implemented instead as the same zero-cost `Int64`-newtype-wrapping-
//! an-opaque-word shape `Regex`/`NativeHandle`/`LogFields` already
//! establish (`emerald-sema`'s `Type::Newtype("Bytes", Int64)`,
//! `emerald-codegen`'s `NEWTYPE_UNDERLYING`/`newtypes` entries) — a
//! single machine word carrying a raw heap pointer (never a
//! `crate::handle` registry id; unlike `Regex`, a `Bytes` value holds
//! no Rust-side resource needing a `Box`/registry entry at all, so a
//! plain pointer is strictly simpler and cheaper) to a heap block laid
//! out `[len: i64][data: u8 * len]` — the identical "length-prefixed
//! heap block, pointer is the whole value" convention `Array[T]`/
//! `Hash[K, V]` already use, just packed byte-tight instead of
//! 8-byte-per-element.
//!
//! Disclosed deviation from the plan's own literal wording: a genuine
//! two-word value would require threading a second machine word
//! through every function-call/struct-field/array-element codegen
//! path in `emerald-codegen` (a far larger blast radius than this
//! plan's real need — a printable, hex-encodable digest buffer, plus
//! plans 110/111's own keys/ciphertexts/signatures); this shape gives
//! the identical ptr+len information behind one pointer instead, at
//! zero cost to every OTHER type's own codegen.

use std::os::raw::c_char;

/// Allocates a fresh `[len: i64][data]` heap block copying `data`,
/// returned as a bare pointer reinterpreted as `Int64` — shared by
/// every domain plan producing a `Bytes` value (this plan's own six
/// hash digests; plans 110/111's keys/ciphertexts/signatures).
pub unsafe fn bytes_from_slice(data: &[u8]) -> i64 {
  let ptr = crate::emerald_alloc(8 + data.len() as i64) as *mut i64;
  *ptr = data.len() as i64;
  let dst = ptr.add(1) as *mut u8;
  std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
  ptr as i64
}

/// Reads a `Bytes` value's own `[len: i64][data]` heap block back into
/// a borrowed `&[u8]` — shared by every domain plan consuming a
/// `Bytes` value as input (this plan's own six hash functions).
///
/// # Safety
/// `id` must be a pointer `bytes_from_slice` (or an equally-shaped
/// native producer) actually returned.
pub unsafe fn bytes_as_slice<'a>(id: i64) -> &'a [u8] {
  let ptr = id as *const i64;
  let len = *ptr as usize;
  let data = ptr.add(1) as *const u8;
  std::slice::from_raw_parts(data, len)
}

/// `String.to_bytes(self): Bytes` — copies `s`'s own bytes (obtained
/// the same `strlen`-via-`CStr` way `emerald_string_length` already
/// does) into a fresh `Bytes` value.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_to_bytes(s: *const c_char) -> i64 {
  if s.is_null() {
    crate::raise_native_error("String.to_bytes: null string pointer");
  }
  let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
  bytes_from_slice(bytes)
}

/// `Bytes.to_hex(self): String` — lower-case hex encoding of the
/// buffer's own bytes, via the `hex` crate already vetted and linked
/// by plan 123 (a real, disclosed improvement over this plan's own
/// suggested "dependency-free nibble-to-ASCII loop" — `hex::encode` is
/// the identical vetted dependency `Hex.encode` already calls, reused
/// rather than a second, hand-rolled implementation of the same
/// encoding).
///
/// # Safety
/// `id` must be a pointer `bytes_from_slice` actually returned.
pub unsafe fn bytes_to_hex(id: i64) -> *const c_char {
  let s = hex::encode(bytes_as_slice(id));
  crate::alloc_and_copy_str(&s)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn round_trips_bytes_from_a_string_and_back_to_hex() {
    unsafe {
      let s = std::ffi::CString::new("hi").unwrap();
      let id = string_to_bytes(s.as_ptr());
      assert_eq!(bytes_as_slice(id), b"hi");
      let hex_ptr = bytes_to_hex(id);
      assert_eq!(std::ffi::CStr::from_ptr(hex_ptr).to_str().unwrap(), "6869");
    }
  }

  #[test]
  fn bytes_from_slice_preserves_an_embedded_nul_byte() {
    unsafe {
      let id = bytes_from_slice(&[0x00, 0xff, 0x10]);
      assert_eq!(bytes_as_slice(id), &[0x00, 0xff, 0x10]);
    }
  }
}
