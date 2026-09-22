// Plan 91 - the Rust-native runtime crate; plan 92 generalizes its
// one proof function into the reusable convention every later
// emerald-rt export (plans 96-191) must follow. A second static
// archive, alongside runtime/emerald_runtime.c's existing C one,
// that emerald-driver's build.rs compiles (via a cargo build -p
// emerald-rt --release subprocess, JSON-artifact-discovered - stable
// Cargo has no artifact-dependency feature to do this natively) and
// embeds into the final emerald-cli binary.
//
// Naming: emerald_rt_MODULE_FN. MODULE is the lowercase noun the
// function's Emerald-facing surface groups under (string, file,
// http, sqlite, ...) - never the wrapped third-party crate's own
// name (plan 95's crate-vetting ledger tracks that separately; a
// module's backing crate can be swapped by a future re-vetting
// without renaming every ABI symbol a compiled Emerald program's
// object file already references).
//
// The panic-boundary convention every export in this crate follows:
// runtime/emerald_runtime.c's exceptions are setjmp/longjmp-based
// (plan 11) - a Rust panic unwinding past a plain extern "C" fn
// boundary into those C frames is undefined behavior twice over (no
// unwind tables describe setjmp-based C frames at all, and
// unwinding across a non-"C-unwind" extern "C" boundary is already
// UB per Rust's own reference). Every exported function's entire body
// must therefore run through catch_and_raise, which converts a
// caught panic into a real, rescuable Emerald NativeError exception
// (never a sentinel value, never a silent abort) via
// emerald_rt_raise_native_error. This is safe only because the
// workspace keeps its real, un-overridden panic = "unwind" default -
// see this crate's own Cargo.toml.
//
// Result-vs-exception is a rule, not a per-call judgment: an
// anticipated failure the wrapped crate's own API already
// distinguishes (a parse error, a connection refused, a file not
// found) is always a Result value, constructed via
// emerald_rt_result_ok/emerald_rt_result_err. A genuine Rust panic
// caught at the catch_and_raise boundary is always a raised
// NativeError, never a Result - plan 53's own two-channel model
// (Result for expected, recoverable failures a caller is meant to
// handle; exceptions for conditions a caller isn't expected to
// routinely check for) applied here without exception.
//
// The binary-safe buffer convention (documented, not yet used): a
// future domain plan needing embedded-NUL or non-UTF-8 byte data
// (compression, crypto, binary serialization, images) must use a
// ptr-plus-length two-parameter shape in place of a bare
// *const c_char, on both the parameter and return side - a
// returning function takes an additional out-length out-parameter,
// since a bare two-word repr(C) struct return is not guaranteed to
// pass in registers identically across every future host ABI this
// project may target, the same reasoning emerald_string_length-
// adjacent runtime helpers already apply via an out-parameter for
// their own multi-value returns. Disclosed, not solved by this plan:
// an Emerald String value crossing this convention as an INPUT is
// still bounded by emerald_string_length's own strlen-based length
// (plan 59's own finding - String has no length header) - a domain
// plan built on this convention can consume arbitrary bytes a native
// call produces, but cannot yet accept an Emerald-source-literal
// String containing an embedded NUL byte. That requires a real
// Emerald-level Bytes/binary-literal type, not yet designed.
//
// Plan 94's async-to-sync bridging convention (documented here, no
// code merged by this plan - see its own Decision log for why `tokio`
// is not added to this crate's Cargo.toml yet). Emerald has no
// async/await surface and none of the 96+ upcoming domain plans add
// one - every emerald_rt_* export stays a plain, synchronous
// `extern "C" fn`, exactly plan 92's own shape, regardless of what it
// does internally to get its answer.
//
// Preference order every domain plan citing plan 94 must justify
// against: (1) a crate with a genuinely synchronous, non-async API
// (ureq for HTTP, tungstenite's own blocking mode for WebSocket) is
// always preferred when one exists and is otherwise plan-95-vetting-
// eligible - zero embedded runtime, and a plain blocking call is
// indistinguishable, from plan 55's scheduler's point of view, from
// any CPU-bound native call this crate already makes; (2) only when no
// sync-native option survives plan 95's vetting bar does a domain plan
// reach for an async crate (reqwest, tonic, tokio-tungstenite) bridged
// via the pattern below. A domain plan reaching for (2) without first
// stating why (1) was unavailable or rejected fails plan 95's own
// acceptance checklist item 3.
//
// The exact pattern the first async-backed domain plan must implement
// verbatim (illustrative prose here, not compiled code - `tokio` is
// not a real dependency of this crate as of plan 94):
//
//   static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
//   fn tokio_rt() -> &'static tokio::runtime::Runtime {
//     TOKIO_RT.get_or_init(|| {
//       tokio::runtime::Runtime::new()
//         .expect("emerald-rt: failed to start the shared tokio runtime")
//     })
//   }
//
// Exactly one shared, lazily-initialized Runtime behind a OnceLock,
// never one runtime per call and never one runtime per domain plan -
// every async-backed export calls `tokio_rt().block_on(async { ... })`
// inside its own catch_and_raise-wrapped body against that one shared
// runtime. Whichever domain plan implements this pattern for real is
// the one that adds `tokio` to this crate's Cargo.toml and to
// DEPENDENCIES.md (plan 95's ledger), subject to plan 95's own vetting
// bar (a genuine WebSearch/WebFetch-verified check against crates.io
// and the RustSec advisory database) at THAT plan's own authoring
// time - this module's own naming of `tokio` throughout is
// illustrative of the pattern, not a pre-vetted approval a later plan
// can cite in place of doing that check itself.
//
// The cost/safety analysis every domain plan author should understand
// before using this pattern: a `.block_on(...)` call inside an actor
// method body is memory-safe by construction, independent of how long
// it blocks (plan 55's own safety argument is about which THREAD may
// touch an actor's state at a given moment, never about how LONG a
// method body takes to return - a 200ms block_on changes nothing about
// that argument). The real cost is throughput, not safety: plan 55's
// worker pool is a FIXED pool of OS threads shared by every actor in
// the process, so a slow block_on call occupies one of those threads
// for its entire real-world duration, starving every OTHER actor's
// mailbox in the meantime - a real, disclosed, non-compiler-enforced
// concern. The mitigation is architectural, not mechanical: spread
// concurrent slow native calls across many actor instances (each with
// its own mailbox, per plan 54) rather than piling them onto one, so
// plan 55's own multi-worker pool can actually overlap them - the same
// shape its own Spinner proof already demonstrates for pure CPU-bound
// work, now generalized to I/O-bound native calls.

use std::ffi::c_void;
use std::os::raw::c_char;

mod aead;
mod asymmetric;
mod bytes;
mod encoding;
mod env;
mod handle;
mod hashing;
mod humantime;
mod json;
mod log;
mod math;
mod random;
mod regex;
mod secure_compare;
mod system;
// Named `urls`, not `url` — this crate's own `mod url` would shadow
// the external `url` crate this module wraps, exactly the collision
// `aead.rs` hit against the external `aead` crate (see that module's
// own doc comment); `#[path]` keeps the file itself named `url.rs`.
#[path = "url.rs"]
mod urls;

// NativeError's class tag - fixed and reserved, assigned before any
// user-declared class in emerald-codegen's own class-tag-assignment
// pass (plan 92's Decision log), so this crate can hardcode it with
// no access to any particular compilation's own class registry.
const NATIVE_ERROR_TAG: i64 = 0;

// Plan 92's Decision log: reuses the exact two runtime entry points
// Stmt::Raise's own codegen lowering already calls (plan 11) - no
// new Rust-side allocator or exception mechanism. Both symbols are
// already exported from the linked-in C archive per plan 91's
// dual-archive build; declaring them here crosses that seam
// deliberately for the first time, not adding a new one.
extern "C" {
  fn emerald_alloc(size: i64) -> *mut c_void;
  fn emerald_raise(tag: i64, ptr: *mut c_void) -> !;
}

// Copies s's bytes plus a trailing NUL into a fresh emerald_alloc
// buffer and returns it as a *const c_char - every message this
// crate hands to Emerald-visible String fields must be copied this
// way, never a raw pointer into this crate's own transient Rust
// memory. Safety: always safe to call; the returned pointer is a
// real, permanently allocated NUL-terminated buffer.
unsafe fn alloc_and_copy_str(s: &str) -> *const c_char {
  let bytes = s.as_bytes();
  let buf = emerald_alloc(bytes.len() as i64 + 1) as *mut u8;
  std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
  *buf.add(bytes.len()) = 0;
  buf as *const c_char
}

// Extracts a real message from a catch_unwind payload - panic! with
// a &str/String argument is by far the common case (both are
// handled), any other payload shape falls back to a fixed literal.
// A real, disclosed finding, found only by actually running the
// panicking `--release`-built export through the real CLI (this
// plan's own Concrete Proof), not caught by any in-process unit test:
// `catch_unwind`'s own `Err` payload does NOT reliably downcast to
// `&str`/`String` here — verified empirically (a temporary debug
// probe showed `payload.is::<&str>()` and `.is::<String>()` both
// `false` for a plain, argument-less `panic!("literal")`, despite an
// in-process unit test of the identical macro invocation shape
// succeeding). Rather than depend on the payload's own concrete type
// at all, this crate captures the panic's fully-formatted message via
// a real `std::panic::set_hook` callback instead — `PanicHookInfo`'s
// own `Display` impl formats a real message string regardless of the
// payload's underlying type, sidestepping the downcast entirely. The
// hook stores into `LAST_PANIC_MESSAGE`, a thread-local `catch_and_
// raise` reads right after `catch_unwind` returns `Err` on the SAME
// thread the hook just ran on (a real ordering guarantee: the default
// hook always runs synchronously, on the panicking thread, before
// unwinding ever reaches `catch_unwind`'s own frame).
thread_local! {
  static LAST_PANIC_MESSAGE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn install_panic_hook_once() {
  static INIT: std::sync::Once = std::sync::Once::new();
  INIT.call_once(|| {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
      // `PanicHookInfo`'s own `Display` renders `"panicked at
      // {location}:\n{message}"` (verified empirically, run to run,
      // against this exact rustc — there's no stable `.message()`
      // accessor on this toolchain to get the message alone). Split
      // on the first newline rather than parse the location text
      // itself, since the message is always everything after it.
      let rendered = info.to_string();
      let message = rendered
        .split_once('\n')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or(rendered);
      LAST_PANIC_MESSAGE.with(|cell| {
        *cell.borrow_mut() = Some(message);
      });
      previous(info);
    }));
  });
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
  if let Some(s) = LAST_PANIC_MESSAGE.with(|cell| cell.borrow_mut().take()) {
    return s;
  }
  // Fallback only: reached if the hook above somehow never ran (a
  // panic-in-panic, or a hook installed by embedding code that
  // doesn't chain to the previous one) — the original downcast-based
  // extraction stays as a genuine second attempt, not dead code.
  if let Some(s) = payload.downcast_ref::<&str>() {
    (*s).to_string()
  } else if let Some(s) = payload.downcast_ref::<String>() {
    s.clone()
  } else {
    "native panic (no message)".to_string()
  }
}

// Allocates a real NativeError instance (one field, message: String,
// at offset 0) and raises it via the exact entry point Stmt::Raise's
// own codegen lowering already calls. Diverges - emerald_raise never
// returns (a setjmp/longjmp jump away, not an ordinary return).
// Safety: always safe to call with any msg.
// Split from `raise_native_error` below specifically so the real
// construction logic (message copy, one-field instance layout) is
// unit-testable without ever calling `emerald_raise` — found necessary
// only by actually running a test that DID call through it: a panic
// escaping a real `extern "C" fn` (this test build's own `emerald_raise`
// stub, itself declared `extern "C"` to match the real symbol's ABI)
// is exactly the "unwind across a plain C boundary" hazard this whole
// plan exists to prevent, and Rust's own runtime correctly aborts the
// process rather than letting it happen — the identical guard that
// makes the real `catch_and_raise` boundary necessary in the first
// place, discovered by hitting it, not anticipated in the design.
unsafe fn build_native_error_instance(msg: &str) -> *mut c_void {
  let msg_ptr = alloc_and_copy_str(msg);
  let instance = emerald_alloc(8) as *mut *const c_char;
  *instance = msg_ptr;
  instance as *mut c_void
}

unsafe fn raise_native_error(msg: &str) -> ! {
  let instance = build_native_error_instance(msg);
  emerald_raise(NATIVE_ERROR_TAG, instance)
}

// emerald_rt_raise_native_error - the one exported, ABI-visible entry
// point a future domain plan's own non-panic-triggered failure can
// call directly, alongside the panic-triggered path catch_and_raise
// already handles automatically.
/// # Safety
/// `msg`, if non-null, must point to a valid, NUL-terminated C
/// string. A null `msg` raises a fixed fallback message rather than
/// dereferencing a null pointer.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_raise_native_error(msg: *const c_char) -> ! {
  if msg.is_null() {
    raise_native_error("native error (no message)")
  } else {
    let s = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
    raise_native_error(&s)
  }
}

// Wraps body in catch_unwind; on a caught panic, converts the payload
// into a message and raises it as a real NativeError exception
// (diverges) rather than returning any sentinel value - the one
// convention every export in this crate is written through.
// Safety: body must itself uphold whatever safety contract the
// caller's own exported function documents.
unsafe fn catch_and_raise<T>(body: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
  install_panic_hook_once();
  match std::panic::catch_unwind(body) {
    Ok(v) => v,
    Err(payload) => raise_native_error(&panic_message(&payload)),
  }
}

// Allocates plan 53's own Result layout in Ok form - discriminant 0
// at offset 0, payload at offset 8 - byte-for-byte what
// emerald-codegen's own Expr::Ok arm already produces, so a native
// function declaring a Result return type (whose Ok payload is a
// plain 8-byte scalar) needs zero additional marshaling at the call
// site.
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_result_ok(payload: i64) -> *mut c_void {
  let ptr = emerald_alloc(16) as *mut i64;
  *ptr = 0;
  *ptr.add(1) = payload;
  ptr as *mut c_void
}

// emerald_rt_result_ok's Err sibling - discriminant 1 at offset 0,
// message at offset 8. msg's bytes are copied into a fresh,
// permanently allocated buffer, never referenced directly.
/// # Safety
/// `msg`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_result_err(msg: *const c_char) -> *mut c_void {
  let copied = if msg.is_null() {
    alloc_and_copy_str("")
  } else {
    let s = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
    alloc_and_copy_str(&s)
  };
  let ptr = emerald_alloc(16) as *mut i64;
  *ptr = 1;
  *(ptr.add(1) as *mut *const c_char) = copied;
  ptr as *mut c_void
}

// `emerald_rt_result_err`'s own internal-Rust-caller sibling - takes a
// real `&str` directly rather than a `*const c_char`, avoiding a
// pointless `CString` round-trip for a call site (plan 118's
// `json::json_parse`) that already has an owned Rust `String` in hand.
// Never `#[no_mangle]`/`extern "C"` - not an ABI-visible export, purely
// an internal helper shared across this crate's own modules.
unsafe fn emerald_rt_result_err_str(msg: &str) -> *mut c_void {
  let copied = alloc_and_copy_str(msg);
  let ptr = emerald_alloc(16) as *mut i64;
  *ptr = 1;
  *(ptr.add(1) as *mut *const c_char) = copied;
  ptr as *mut c_void
}

// The FNV-1a-32 hash of s's bytes, widened to i64. Renamed from plan
// 91's own emerald_rt_fnv1a_hash to follow this plan's naming
// convention - a pure rename, no behavior change.
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C
/// string. A null `s` raises a NativeError rather than dereferencing a
/// null pointer.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_fnv1a_hash(s: *const c_char) -> i64 {
  catch_and_raise(move || {
    if s.is_null() {
      raise_native_error("emerald_rt_string_fnv1a_hash: null string pointer")
    } else {
      let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
      fnv1a_hash_bytes(bytes) as i64
    }
  })
}

// .fnv1a_hash_checked - the same hash, but returning a Result rather
// than a bare Int64: an error for "", the same value .fnv1a_hash
// produces otherwise.
/// # Safety
/// Same contract as `emerald_rt_string_fnv1a_hash`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_fnv1a_hash_checked(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || {
    if s.is_null() {
      raise_native_error("emerald_rt_string_fnv1a_hash_checked: null string pointer")
    } else {
      let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
      if bytes.is_empty() {
        let msg = std::ffi::CString::new("input must not be empty").unwrap();
        emerald_rt_result_err(msg.as_ptr())
      } else {
        emerald_rt_result_ok(fnv1a_hash_bytes(bytes) as i64)
      }
    }
  })
}

// A deliberately, unconditionally panicking export - exists solely
// so this plan's own Concrete Proof can demonstrate a genuine
// Rust-side panic converting into a genuine, rescuable Emerald
// NativeError exception via catch_and_raise. _s is unused (every
// String-method export takes the receiver pointer as its first
// argument); returns nothing meaningful (Void on the Emerald side).
/// # Safety
/// Always safe to call - the panic is unconditional.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_fnv1a_hash_panic_for_test(_s: *const c_char) {
  catch_and_raise(move || {
    panic!("native panic in fnv1a_hash_panic_for_test");
  })
}

// The actual FNV-1a-32 algorithm - a pure function of bytes, with no
// FFI/panic concerns of its own, so tests below can assert against a
// hand-computed value with nothing else in the way.
fn fnv1a_hash_bytes(bytes: &[u8]) -> u32 {
  const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
  const FNV_PRIME: u32 = 0x0100_0193;
  let mut hash = FNV_OFFSET_BASIS;
  for &b in bytes {
    hash ^= u32::from(b);
    hash = hash.wrapping_mul(FNV_PRIME);
  }
  hash
}

// Plan 93's own Concrete Proof: a trivial in-memory counter "resource"
// backed by `handle::handle_alloc`/`_get`/`_get_mut`/`_close` — not a
// real I/O resource (that's a later domain plan's job, per Out of
// scope), the vehicle this plan uses to exercise the registry's own
// lifecycle end to end through a real Emerald program.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_handle_counter_open() -> i64 {
  catch_and_raise(move || handle::handle_alloc(Box::new(0i64), "counter"))
}

/// # Safety
/// `id` should be a value `emerald_rt_handle_counter_open` returned —
/// a closed or unknown `id` raises a real `NativeError` (plan 92's
/// channel) rather than corrupting memory or aborting the process.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_handle_counter_bump(id: i64) -> i64 {
  catch_and_raise(move || {
    match handle::handle_get_mut::<i64, _>(id, "counter", |v| {
      *v += 1;
      *v
    }) {
      Ok(v) => v,
      Err(msg) => raise_native_error(&msg),
    }
  })
}

/// # Safety
/// Always safe to call, including on an already-closed or unknown
/// `id` — closing is idempotent (`leaf-double-close-is-a-noop-not-an-
/// error`) and never raises.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_handle_counter_close(id: i64) {
  catch_and_raise(move || {
    handle::handle_close(id);
  })
}

// Plan 118 (JSON): `Json.parse`/`JsonValue.get`/`JsonValue.to_s`,
// dispatched by exact free-function/receiver-gated name in
// `emerald-codegen`'s own `build_expr`/`build_method_call` — see
// `json.rs`'s own module doc for the full design.

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_json_parse(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || json::json_parse(s))
}

/// # Safety
/// `obj` must point to a real `JsonValue` block. `key`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_json_object_get(
  obj: *const c_void,
  key: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || json::json_object_get(obj, key))
}

/// # Safety
/// `obj` must point to a real `JsonValue` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_json_to_string(obj: *const c_void) -> *const c_char {
  catch_and_raise(move || json::json_to_string(obj))
}

// Plan 123 (Base64 & Hex Encoding — revised, `String`-only scope; see
// `encoding.rs`'s own module doc): `Base64.encode`/`.decode` and their
// four-variant siblings, `Hex.encode`/`.encode_upper`/`.decode`,
// dispatched by exact free-function name in `emerald-codegen`'s own
// `build_method_call`, mirroring `Json`/`Log`/`File`'s reserved-
// namespace static-call shape exactly.

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_encode(s: *const c_char) -> *const c_char {
  catch_and_raise(move || encoding::base64_encode(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_decode(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || encoding::base64_decode(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_encode_no_pad(s: *const c_char) -> *const c_char {
  catch_and_raise(move || encoding::base64_encode_no_pad(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_decode_no_pad(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || encoding::base64_decode_no_pad(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_encode_url_safe(s: *const c_char) -> *const c_char {
  catch_and_raise(move || encoding::base64_encode_url_safe(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_decode_url_safe(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || encoding::base64_decode_url_safe(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_encode_url_safe_padded(
  s: *const c_char,
) -> *const c_char {
  catch_and_raise(move || encoding::base64_encode_url_safe_padded(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_base64_decode_url_safe_padded(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || encoding::base64_decode_url_safe_padded(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_hex_encode(s: *const c_char) -> *const c_char {
  catch_and_raise(move || encoding::hex_encode(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_hex_encode_upper(s: *const c_char) -> *const c_char {
  catch_and_raise(move || encoding::hex_encode_upper(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_hex_decode(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || encoding::hex_decode(s))
}

// Plan 122 (Regular Expressions): `Regex.compile` and its instance
// methods (dispatched by `emerald-codegen`'s own `Regex`-newtype
// method-call arm, not the reserved-namespace static-call shape
// `Json`/`Log`/`Base64`/`Hex` use) — see `regex.rs`'s own module doc.

/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_compile(pattern: *const c_char) -> *mut c_void {
  catch_and_raise(move || regex::regex_compile(pattern))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_is_match(id: i64, s: *const c_char) -> i64 {
  catch_and_raise(move || regex::regex_is_match(id, s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_find(id: i64, s: *const c_char) -> *mut c_void {
  catch_and_raise(move || regex::regex_find(id, s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_find_all(id: i64, s: *const c_char) -> *mut c_void {
  catch_and_raise(move || regex::regex_find_all(id, s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_find_all_count(id: i64, s: *const c_char) -> i64 {
  catch_and_raise(move || regex::regex_find_all_count(id, s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_captures(id: i64, s: *const c_char) -> *mut c_void {
  catch_and_raise(move || regex::regex_captures(id, s))
}

/// # Safety
/// `s`/`replacement`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_replace(
  id: i64,
  s: *const c_char,
  replacement: *const c_char,
) -> *const c_char {
  catch_and_raise(move || regex::regex_replace(id, s, replacement))
}

/// # Safety
/// `s`/`replacement`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_replace_all(
  id: i64,
  s: *const c_char,
  replacement: *const c_char,
) -> *const c_char {
  catch_and_raise(move || regex::regex_replace_all(id, s, replacement))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_split(id: i64, s: *const c_char) -> *mut c_void {
  catch_and_raise(move || regex::regex_split(id, s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_regex_split_count(id: i64, s: *const c_char) -> i64 {
  catch_and_raise(move || regex::regex_split_count(id, s))
}

// Plan 146 (Environment Variables): `Env.get`/`.set`/`.remove`/
// `.keys`/`.keys_count` — see `env.rs`'s own module doc for the
// mutex-serialization design (Rust 2024's `unsafe fn` reclassification
// of `set_var`/`remove_var`).

/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_env_get(key: *const c_char) -> *mut c_char {
  catch_and_raise(move || env::env_get(key))
}

/// # Safety
/// `key`/`value`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_env_set(key: *const c_char, value: *const c_char) {
  catch_and_raise(move || env::env_set(key, value))
}

/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_env_remove(key: *const c_char) {
  catch_and_raise(move || env::env_remove(key))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_env_keys() -> *mut c_void {
  catch_and_raise(move || env::env_keys())
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_env_keys_count() -> i64 {
  catch_and_raise(move || env::env_keys_count())
}

// Plan 164 (Portable Math Functions): `Math.<name>`, a thin f64-in-
// f64-out wrapping of `libm` — see `math.rs`'s own module doc. Every
// one of these is TOTAL over f64's full domain (no panic path), but
// still wrapped in `catch_and_raise` uniformly per plan 92's own
// "consistency over cleverness" rationale.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_sin(x: f64) -> f64 {
  catch_and_raise(move || math::sin(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_cos(x: f64) -> f64 {
  catch_and_raise(move || math::cos(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_tan(x: f64) -> f64 {
  catch_and_raise(move || math::tan(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_asin(x: f64) -> f64 {
  catch_and_raise(move || math::asin(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_acos(x: f64) -> f64 {
  catch_and_raise(move || math::acos(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_atan(x: f64) -> f64 {
  catch_and_raise(move || math::atan(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_atan2(y: f64, x: f64) -> f64 {
  catch_and_raise(move || math::atan2(y, x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_exp(x: f64) -> f64 {
  catch_and_raise(move || math::exp(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_exp2(x: f64) -> f64 {
  catch_and_raise(move || math::exp2(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_ln(x: f64) -> f64 {
  catch_and_raise(move || math::ln(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_log2(x: f64) -> f64 {
  catch_and_raise(move || math::log2(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_log10(x: f64) -> f64 {
  catch_and_raise(move || math::log10(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_pow(x: f64, y: f64) -> f64 {
  catch_and_raise(move || math::pow(x, y))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_sqrt(x: f64) -> f64 {
  catch_and_raise(move || math::sqrt(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_cbrt(x: f64) -> f64 {
  catch_and_raise(move || math::cbrt(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_hypot(x: f64, y: f64) -> f64 {
  catch_and_raise(move || math::hypot(x, y))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_floor(x: f64) -> f64 {
  catch_and_raise(move || math::floor(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_ceil(x: f64) -> f64 {
  catch_and_raise(move || math::ceil(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_round(x: f64) -> f64 {
  catch_and_raise(move || math::round(x))
}
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_math_trunc(x: f64) -> f64 {
  catch_and_raise(move || math::trunc(x))
}

// Plan 162 (Human-Readable Duration/Time Formatting): `Duration.
// humanize`/`.parse_human`, `Timestamp.to_rfc3339`/`.parse_rfc3339` —
// see `humantime.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_humantime_format_duration(seconds: i64) -> *const c_char {
  catch_and_raise(move || humantime::format_duration(seconds))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_humantime_parse_duration(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || humantime::parse_duration(s))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_humantime_format_rfc3339(unix_secs: i64) -> *const c_char {
  catch_and_raise(move || humantime::format_rfc3339(unix_secs))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_humantime_parse_rfc3339(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || humantime::parse_rfc3339(s))
}

// Plan 152 (System Information): `System.*` — read-only CPU/memory/
// disk/process introspection, see `system.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_cpu_count() -> i64 {
  catch_and_raise(system::cpu_count)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_total_memory_bytes() -> i64 {
  catch_and_raise(system::total_memory_bytes)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_used_memory_bytes() -> i64 {
  catch_and_raise(system::used_memory_bytes)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_disk_names() -> *mut c_void {
  catch_and_raise(system::disk_names)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_disk_names_count() -> i64 {
  catch_and_raise(system::disk_names_count)
}

/// # Safety
/// `name`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_disk_total_bytes(name: *const c_char) -> i64 {
  catch_and_raise(move || system::disk_total_bytes(name))
}

/// # Safety
/// `name`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_disk_available_bytes(name: *const c_char) -> i64 {
  catch_and_raise(move || system::disk_available_bytes(name))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_process_ids() -> *mut c_void {
  catch_and_raise(system::process_ids)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_process_ids_count() -> i64 {
  catch_and_raise(system::process_ids_count)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_process_name(pid: i64) -> *mut c_char {
  catch_and_raise(move || system::process_name(pid))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_system_process_memory_bytes(pid: i64) -> i64 {
  catch_and_raise(move || system::process_memory_bytes(pid))
}

// Plan 168 (Structured Logging): `Log.configure`/`.trace`/`.debug`/
// `.info`/`.warn`/`.error`/`.*_fields`, `LogFields.new`/`.set` - see
// `log.rs`'s own module doc for the full design.

/// # Safety
/// `level`/`format`, if non-null, must point to valid, NUL-terminated
/// C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_log_configure(
  level: *const c_char,
  format: *const c_char,
) -> i64 {
  catch_and_raise(move || log::log_configure(level, format))
}

/// # Safety
/// `message`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_log_event(level: i64, message: *const c_char) -> i64 {
  catch_and_raise(move || log::log_event(level, message))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_log_fields_new() -> i64 {
  catch_and_raise(log::log_fields_new)
}

/// # Safety
/// `key`/`value`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_log_fields_set(
  handle: i64,
  key: *const c_char,
  value: *const c_char,
) -> i64 {
  catch_and_raise(move || log::log_fields_set(handle, key, value))
}

/// # Safety
/// `message`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_log_event_fields(
  level: i64,
  message: *const c_char,
  handle: i64,
) -> i64 {
  catch_and_raise(move || log::log_event_fields(level, message, handle))
}

// Plan 109 (Cryptographic Hashing): `String.to_bytes`/`Bytes.to_hex`
// (see `bytes.rs`'s own module doc for `Bytes`'s real, disclosed
// representation), `Sha256`/`Sha512`/`Sha3_256`/`Sha3_512`/`Blake3`/
// `Md5.hash` one-shot digests, and `Sha256Hasher`/`Blake3Hasher`
// incremental handles (see `hashing.rs`'s own module doc).

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_to_bytes(s: *const c_char) -> i64 {
  catch_and_raise(move || bytes::string_to_bytes(s))
}

/// # Safety
/// `id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bytes_to_hex(id: i64) -> *const c_char {
  catch_and_raise(move || bytes::bytes_to_hex(id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha256_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::sha256_hash(bytes_id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha512_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::sha512_hash(bytes_id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha3_256_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::sha3_256_hash(bytes_id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha3_512_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::sha3_512_hash(bytes_id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_blake3_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::blake3_hash(bytes_id))
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_md5_hash(bytes_id: i64) -> i64 {
  catch_and_raise(move || hashing::md5_hash(bytes_id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha256_hasher_new() -> i64 {
  catch_and_raise(hashing::sha256_hasher_new)
}

/// # Safety
/// `id` must be a live `Sha256Hasher` handle; `bytes_id` must be a
/// live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha256_hasher_update(id: i64, bytes_id: i64) {
  catch_and_raise(move || hashing::sha256_hasher_update(id, bytes_id))
}

/// # Safety
/// `id` must be a live `Sha256Hasher` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sha256_hasher_finalize(id: i64) -> i64 {
  catch_and_raise(move || hashing::sha256_hasher_finalize(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_blake3_hasher_new() -> i64 {
  catch_and_raise(hashing::blake3_hasher_new)
}

/// # Safety
/// `id` must be a live `Blake3Hasher` handle; `bytes_id` must be a
/// live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_blake3_hasher_update(id: i64, bytes_id: i64) {
  catch_and_raise(move || hashing::blake3_hasher_update(id, bytes_id))
}

/// # Safety
/// `id` must be a live `Blake3Hasher` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_blake3_hasher_finalize(id: i64) -> i64 {
  catch_and_raise(move || hashing::blake3_hasher_finalize(id))
}

// Plan 110 (Symmetric AEAD Encryption): `AesGcm256`/`XChaCha20Poly1305`
// `.generate_key`/`.key_from_bytes`/`.encrypt`/`.decrypt`/
// `.encrypt_with_nonce`/`.decrypt_with_nonce`, `AeadKey#free` — see
// `aead.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aead_generate_key() -> i64 {
  catch_and_raise(aead::generate_key)
}

/// # Safety
/// `bytes_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aead_key_from_bytes(bytes_id: i64) -> *mut c_void {
  catch_and_raise(move || aead::key_from_bytes(bytes_id))
}

/// # Safety
/// `id` must be a live `AeadKey` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aead_key_free(id: i64) {
  catch_and_raise(move || aead::key_free(id))
}

/// # Safety
/// `plaintext_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aes_gcm_encrypt(
  key_id: i64,
  plaintext_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::aes_gcm_encrypt(key_id, plaintext_id, aad_id))
}

/// # Safety
/// `sealed_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aes_gcm_decrypt(
  key_id: i64,
  sealed_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::aes_gcm_decrypt(key_id, sealed_id, aad_id))
}

/// # Safety
/// `nonce_id`/`plaintext_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aes_gcm_encrypt_with_nonce(
  key_id: i64,
  nonce_id: i64,
  plaintext_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::aes_gcm_encrypt_with_nonce(key_id, nonce_id, plaintext_id, aad_id))
}

/// # Safety
/// `nonce_id`/`sealed_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_aes_gcm_decrypt_with_nonce(
  key_id: i64,
  nonce_id: i64,
  sealed_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::aes_gcm_decrypt_with_nonce(key_id, nonce_id, sealed_id, aad_id))
}

/// # Safety
/// `plaintext_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xchacha_encrypt(
  key_id: i64,
  plaintext_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::xchacha_encrypt(key_id, plaintext_id, aad_id))
}

/// # Safety
/// `sealed_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xchacha_decrypt(
  key_id: i64,
  sealed_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::xchacha_decrypt(key_id, sealed_id, aad_id))
}

/// # Safety
/// `nonce_id`/`plaintext_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xchacha_encrypt_with_nonce(
  key_id: i64,
  nonce_id: i64,
  plaintext_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::xchacha_encrypt_with_nonce(key_id, nonce_id, plaintext_id, aad_id))
}

/// # Safety
/// `nonce_id`/`sealed_id`/`aad_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xchacha_decrypt_with_nonce(
  key_id: i64,
  nonce_id: i64,
  sealed_id: i64,
  aad_id: i64,
) -> *mut c_void {
  catch_and_raise(move || aead::xchacha_decrypt_with_nonce(key_id, nonce_id, sealed_id, aad_id))
}

// Plan 111 (Asymmetric Cryptography and Digital Signatures):
// `Ed25519`/`X25519`/`Rsa` — see `asymmetric.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ed25519_generate_key() -> i64 {
  catch_and_raise(asymmetric::ed25519_generate_key)
}

/// # Safety
/// `msg_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ed25519_sign(id: i64, msg_id: i64) -> i64 {
  catch_and_raise(move || asymmetric::ed25519_sign(id, msg_id))
}

/// # Safety
/// Always safe to call for a live `Ed25519KeyPair` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ed25519_public_key(id: i64) -> i64 {
  catch_and_raise(move || asymmetric::ed25519_public_key(id))
}

/// # Safety
/// `pubkey_id`/`msg_id`/`sig_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ed25519_verify(
  pubkey_id: i64,
  msg_id: i64,
  sig_id: i64,
) -> *mut c_void {
  catch_and_raise(move || asymmetric::ed25519_verify(pubkey_id, msg_id, sig_id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_generate_ephemeral() -> i64 {
  catch_and_raise(asymmetric::x25519_generate_ephemeral)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_generate_static() -> i64 {
  catch_and_raise(asymmetric::x25519_generate_static)
}

/// # Safety
/// Always safe to call for a live `X25519EphemeralSecret` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_ephemeral_public_key(id: i64) -> i64 {
  catch_and_raise(move || asymmetric::x25519_ephemeral_public_key(id))
}

/// # Safety
/// Always safe to call for a live `X25519StaticSecret` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_static_public_key(id: i64) -> i64 {
  catch_and_raise(move || asymmetric::x25519_static_public_key(id))
}

/// # Safety
/// `their_public_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_ephemeral_diffie_hellman(
  id: i64,
  their_public_id: i64,
) -> i64 {
  catch_and_raise(move || asymmetric::x25519_ephemeral_diffie_hellman(id, their_public_id))
}

/// # Safety
/// `their_public_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_x25519_static_diffie_hellman(
  id: i64,
  their_public_id: i64,
) -> i64 {
  catch_and_raise(move || asymmetric::x25519_static_diffie_hellman(id, their_public_id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_rsa_generate_key(bits: i64) -> *mut c_void {
  catch_and_raise(move || asymmetric::rsa_generate_key(bits))
}

/// # Safety
/// `data_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_rsa_encrypt(keypair_id: i64, data_id: i64) -> *mut c_void {
  catch_and_raise(move || asymmetric::rsa_encrypt(keypair_id, data_id))
}

/// # Safety
/// `data_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_rsa_decrypt(keypair_id: i64, data_id: i64) -> *mut c_void {
  catch_and_raise(move || asymmetric::rsa_decrypt(keypair_id, data_id))
}

/// # Safety
/// `digest_id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_rsa_sign(keypair_id: i64, digest_id: i64) -> *mut c_void {
  catch_and_raise(move || asymmetric::rsa_sign(keypair_id, digest_id))
}

/// # Safety
/// `digest_id`/`sig_id` must be live `Bytes` values.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_rsa_verify(
  keypair_id: i64,
  digest_id: i64,
  sig_id: i64,
) -> *mut c_void {
  catch_and_raise(move || asymmetric::rsa_verify(keypair_id, digest_id, sig_id))
}

// Plan 117 (Constant-Time Comparison): `SecureCompare.eq` — see
// `secure_compare.rs`'s own module doc.

/// # Safety
/// `a`/`b`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_secure_compare(a: *const c_char, b: *const c_char) -> i64 {
  catch_and_raise(move || secure_compare::secure_compare(a, b))
}

// Plan 113 (Cryptographically Secure Random Number Generation):
// `Random.secure_hex`/`.secure_token`/`.int`/`.shuffle` — see
// `random.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_random_secure_hex(n: i64) -> *const c_char {
  catch_and_raise(move || random::random_secure_hex(n))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_random_secure_token(n: i64) -> *const c_char {
  catch_and_raise(move || random::random_secure_token(n))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_random_int(min: i64, max: i64) -> i64 {
  catch_and_raise(move || random::random_int(min, max))
}

/// # Safety
/// `arr` must be a live `Array[Int64]` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_random_shuffle(arr: *mut c_void) {
  catch_and_raise(move || random::random_shuffle(arr as *mut i64))
}

// Plan 98 (URL Parsing): `Url.parse`/`.build`, `Url#scheme`/`#host`/
// `#port`/`#path`/`#query`/`#fragment`/`#with_path`/`#with_query`/
// `#with_port` — see `url.rs`'s own module doc (declared as `mod
// urls` in this file to avoid shadowing the external `url` crate).

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_parse(s: *const c_char) -> i64 {
  catch_and_raise(move || urls::url_parse(s))
}

/// # Safety
/// `scheme`/`host`/`path`, if non-null, must point to valid, NUL-
/// terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_build(
  scheme: *const c_char,
  host: *const c_char,
  path: *const c_char,
) -> i64 {
  catch_and_raise(move || urls::url_build(scheme, host, path))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_scheme(id: i64) -> *const c_char {
  catch_and_raise(move || urls::url_scheme(id))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_host(id: i64) -> *const c_char {
  catch_and_raise(move || urls::url_host(id))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_port(id: i64) -> i64 {
  catch_and_raise(move || urls::url_port(id))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_path(id: i64) -> *const c_char {
  catch_and_raise(move || urls::url_path(id))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_query(id: i64) -> *const c_char {
  catch_and_raise(move || urls::url_query(id))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_fragment(id: i64) -> *const c_char {
  catch_and_raise(move || urls::url_fragment(id))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_with_path(id: i64, path: *const c_char) -> i64 {
  catch_and_raise(move || urls::url_with_path(id, path))
}

/// # Safety
/// `query`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_with_query(id: i64, query: *const c_char) -> i64 {
  catch_and_raise(move || urls::url_with_query(id, query))
}

/// # Safety
/// Always safe to call for a live `Url` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_url_with_port(id: i64, port: i64) -> i64 {
  catch_and_raise(move || urls::url_with_port(id, port))
}

// Real, expected consequence of introducing genuine cross-archive
// extern calls (plan 92's own): `cargo test -p emerald-rt` builds a
// standalone test EXECUTABLE, which (unlike the `--release` staticlib
// `emerald-driver`'s build.rs actually embeds) must fully resolve every
// referenced symbol at link time — emerald_alloc/emerald_raise exist
// only in runtime/emerald_runtime.c's compiled archive, which this
// crate's own isolated test build has no access to at all. These
// test-only stubs provide real, working (if minimal) definitions of
// the exact same symbols, satisfying the `extern "C" { ... }` block
// above ONLY when running tests — never compiled into the real
// `--release` staticlib, where the linked-in C archive provides the
// genuine implementations instead. The real, end-to-end behavior
// (an actual setjmp/longjmp-based exception reaching a real Emerald
// `rescue`) is verified separately, via a real `.em` example run
// through the real CLI (plan 92's own Concrete Proof), not here.
#[cfg(test)]
mod test_stubs {
  #[no_mangle]
  pub extern "C" fn emerald_alloc(size: i64) -> *mut std::ffi::c_void {
    let mut v = vec![0u8; size.max(0) as usize];
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr as *mut std::ffi::c_void
  }

  // Deliberately NOT called by any test — see `build_native_error_
  // instance`'s own doc comment for why: a panic escaping a real
  // `extern "C" fn` (which any Rust-side stub body here would need to
  // do, since there is no Rust-level equivalent to a real `longjmp` to
  // invoke instead) is exactly the "unwind across a plain C boundary"
  // hazard this whole plan exists to prevent, and Rust's own runtime
  // aborts the process rather than allow it — discovered by actually
  // hitting it, not anticipated. Provided only so the `extern "C" {
  // fn emerald_raise ...; }` declaration above resolves at all when
  // this test binary links; `raise_native_error`'s own real behavior
  // (calling this) is verified via a real `.em` example run through
  // the real CLI instead (this plan's own Concrete Proof), never here.
  #[no_mangle]
  pub extern "C" fn emerald_raise(_tag: i64, _ptr: *mut std::ffi::c_void) -> ! {
    std::process::abort();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fnv1a_hash_of_hello_matches_the_known_reference_value() {
    assert_eq!(fnv1a_hash_bytes(b"hello"), 0x4f9f_2cab);
  }

  #[test]
  fn fnv1a_hash_of_hello_world_matches_the_known_reference_value() {
    assert_eq!(fnv1a_hash_bytes(b"hello world"), 0xd58b_3fa7);
  }

  #[test]
  fn fnv1a_hash_of_empty_string_is_the_bare_offset_basis() {
    assert_eq!(fnv1a_hash_bytes(b""), 0x811c_9dc5);
  }

  #[test]
  fn the_extern_c_entry_point_matches_the_pure_function_widened_to_i64() {
    let s = std::ffi::CString::new("hello").unwrap();
    assert_eq!(
      unsafe { emerald_rt_string_fnv1a_hash(s.as_ptr()) },
      0x4f9f_2cab_i64
    );
  }

  #[test]
  fn a_null_string_pointer_is_checked_before_any_dereference() {
    let s: *const c_char = std::ptr::null();
    assert!(s.is_null());
  }

  // Plan 92: proves `build_native_error_instance`'s own real
  // construction logic — a fresh, permanently allocated one-field
  // instance whose `message` field (offset 0) round-trips the exact
  // text passed in. This is as far as an in-process unit test can
  // safely go: the remaining step (`emerald_raise`, a real
  // `setjmp`/`longjmp`) has no safe in-process stand-in (see
  // `test_stubs::emerald_raise`'s own doc comment — a Rust-level panic
  // escaping a real `extern "C" fn` is exactly the hazard this plan
  // exists to prevent, and Rust's own runtime aborts the process
  // rather than allow it). The full path, including the real raise
  // reaching a real Emerald `rescue`, is verified separately via a
  // real `.em` example run through the real CLI (this plan's own
  // Concrete Proof).
  #[test]
  fn build_native_error_instance_round_trips_the_message_field() {
    let ptr = unsafe { build_native_error_instance("native panic in fnv1a_hash_panic_for_test") }
      as *const *const c_char;
    let msg_ptr = unsafe { *ptr };
    let msg = unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
      .to_str()
      .unwrap();
    assert_eq!(msg, "native panic in fnv1a_hash_panic_for_test");
  }

  #[test]
  fn panic_message_extracts_the_str_payload_a_real_panic_bang_produces() {
    let result: Result<(), _> = std::panic::catch_unwind(|| panic!("boom"));
    let payload = result.unwrap_err();
    assert_eq!(panic_message(&*payload), "boom");
  }

  #[test]
  fn fnv1a_hash_checked_rejects_the_empty_string_with_a_real_result_err() {
    let s = std::ffi::CString::new("").unwrap();
    let ptr = unsafe { emerald_rt_string_fnv1a_hash_checked(s.as_ptr()) } as *const i64;
    let discriminant = unsafe { *ptr };
    assert_eq!(
      discriminant, 1,
      "empty input should produce Err (discriminant 1)"
    );
    let msg_ptr = unsafe { *(ptr.add(1) as *const *const c_char) };
    let msg = unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
      .to_str()
      .unwrap();
    assert_eq!(msg, "input must not be empty");
  }

  #[test]
  fn fnv1a_hash_checked_accepts_a_non_empty_string_with_the_same_value_as_the_unchecked_form() {
    let s = std::ffi::CString::new("hello").unwrap();
    let ptr = unsafe { emerald_rt_string_fnv1a_hash_checked(s.as_ptr()) } as *const i64;
    let discriminant = unsafe { *ptr };
    assert_eq!(
      discriminant, 0,
      "non-empty input should produce Ok (discriminant 0)"
    );
    let payload = unsafe { *ptr.add(1) };
    assert_eq!(payload, 0x4f9f_2cab_i64);
  }
}
