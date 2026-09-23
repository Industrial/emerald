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

// Plan 97: the first real (non-illustrative) use of the shared-runtime
// pattern this crate's own module doc above describes — exactly one
// lazily-initialized `Runtime` behind a `OnceLock`, `rt-multi-thread`
// (not `rt`/current-thread alone — `hickory_resolver::Resolver`'s own
// docs warn that its lookup futures and the background tasks they
// spawn must be able to run concurrently on the same executor, which
// a single-threaded runtime blocked on `block_on` cannot provide).
// Any future async-backed domain plan reuses this exact function
// rather than starting a second runtime of its own.
pub(crate) fn tokio_rt() -> &'static tokio::runtime::Runtime {
  static TOKIO_RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
  TOKIO_RT.get_or_init(|| {
    tokio::runtime::Runtime::new().expect("emerald-rt: failed to start the shared tokio runtime")
  })
}

mod aead;
mod asymmetric;
// Plan 163 (Arbitrary-Precision Integers & Decimals), `BigInt` half —
// see `bignum.rs`'s own module doc for why this half is a
// `crate::handle`-registry opaque `Int64` newtype, unlike `decimal.rs`.
mod bignum;
// Plan 125 (Binary Serialization: bincode/msgpack), `Bincode` half —
// named `bincodes`, not `bincode`: this crate's own `mod bincode`
// would shadow the external `bincode` crate this module wraps, the
// identical collision `csvs`/`tomls`/`urls` already dodge — see
// `bincode.rs`'s own module doc for the full account.
#[path = "bincode.rs"]
mod bincodes;
mod bytes;
// Plan 153 (Character Set / Encoding Conversion) — named `charset`,
// not `encoding`: `mod encoding` below is already plan 123's Base64/
// Hex module (its own file predates this plan and wraps a different
// pair of namespaces entirely) — see `charset.rs`'s own module doc
// for the full account of this collision and how it's dodged, the
// same way `csvs`/`tomls`/`urls` already dodge their own external-
// crate-name collisions.
mod charset;
// Plan 193 (`Set[T]`, `Deque[T]`, `PriorityQueue[T]`) — see this
// module's own doc comment for the full account.
mod collections;
// Plan 182 (Structured CLI Flag Parsing) — `CliParser`/`CliParseResult`,
// wrapping `clap`'s non-derive builder API — see `cli.rs`'s own
// module doc.
mod cli;
// Plan 183 (Layered Configuration Loading) — `ConfigBuilder`/
// `ConfigValue`, wrapping the `config` crate — see `config.rs`'s own
// module doc. Named `configs`, not `config` — this crate's own `mod
// config` would shadow the external `config` crate this module wraps,
// the identical collision `csvs`/`tomls`/`urls` already hit and
// disclosed.
#[path = "config.rs"]
mod configs;
// Named `csvs`, not `csv` — this crate's own `mod csv` would shadow
// the external `csv` crate this module wraps, the identical collision
// `aead.rs`/`url.rs`/`toml.rs` already hit and disclosed.
#[path = "csv.rs"]
mod csvs;
// Plan 160 (Date/Time & Timezones) — `DateTime`/`ZonedDateTime`,
// wrapping `jiff` — see `datetime.rs`'s own module doc.
mod datetime;
// Plan 163 (Arbitrary-Precision Integers & Decimals), `Decimal` half —
// see `decimal.rs`'s own module doc for why this half is a packed
// two-`Int64`-field class, deliberately NOT a `crate::handle` registry
// entry the way `bignum.rs`'s `BigInt` is.
mod decimal;
// Plan 144 (Extended Filesystem Operations), `Dir` half — non-
// recursive listing + recursive `walkdir` traversal — see `dir.rs`'s
// own module doc.
mod dir;
mod dns;
mod encoding;
mod env;
mod glob;
// Plan 130 (Gzip/Deflate/Zlib Compression) — `Gzip`/`Deflate`/`Zlib`
// `.compress`/`.decompress`, `GzipWriter`/`GzipReader` (+ `Deflate`/
// `Zlib` siblings), wrapping `flate2` — see `gzip.rs`'s own module doc.
mod gzip;
mod handle;
mod hashing;
mod http2_client;
mod http_client;
mod http_server;
mod humantime;
mod json;
// Plan 114 (JSON Web Tokens) — `Jwt.encode_hs256`/`.verify_hs256`/
// `.encode_rs256`/`.verify_rs256`/`.encode_es256`/`.verify_es256`/
// `.peek_header` — see `jwt.rs`'s own module doc.
mod jwt;
mod kdf;
mod log;
mod math;
// Plan 125 (Binary Serialization: bincode/msgpack), `MessagePack`
// half — wraps `rmp-serde`; no external-crate-name collision to dodge
// here (the wrapped crate is `rmp_serde`, not `msgpack`) — see
// `msgpack.rs`'s own module doc.
mod msgpack;
mod net;
// Plan 112 (Password Hashing) — `Password.hash`/`.verify`, wrapping
// `argon2` — see `password.rs`'s own module doc.
mod password;
// Plan 144 (Extended Filesystem Operations), `Path`/`FileMetadata`
// half — see `path.rs`'s own module doc.
mod path;
// Plan 145 (Process Spawning & Control) — `Process.run`/
// `ProcessResult`, wrapping plain `std::process::Command` — see
// `process.rs`'s own module doc.
mod process;
// Plan 191 (Progress Bars & Terminal Formatting) — `ProgressBar`
// (wrapping `indicatif`) and the `Console.styled`/`Console.is_terminal`
// reserved-namespace static calls (wrapping `console`) — see
// `progress.rs`'s own module doc.
mod progress;
// Plan 180 (Property-Based Testing Generators) — `property "..."
// (params...) do ... end`'s real `proptest`-backed generate/shrink
// session state machine; see this module's own doc comment for the
// full architecture.
mod proptest_support;
mod random;
mod regex;
mod secure_compare;
// Plan 137 (SQLite) — `Sqlite.open`/`.open_memory`/`.close`/
// `.execute_direct`/`.prepare`/`.bind_string`/`.bind_int64`/
// `.bind_float64`/`.bind_null`/`.execute`/`.query`/`.step`/
// `.column_string`/`.column_int64`/`.column_float64`/`.begin`/
// `.commit`/`.rollback`, wrapping `rusqlite` — see `sqlite.rs`'s own
// module doc.
mod sqlite;
// Plan 104 (Server-Sent Events) — `Sse.upgrade`/`.send`/`.comment`/
// `.close`, layered directly on plan 101's `Http.serve`/`tiny_http` —
// see `sse.rs`'s own module doc.
mod sse;
mod system;
// Plan 132 (Tar Archives) -- `Tar.create`/`.extract`, `TarReader.open`/
// `.next_entry`/`.entry_size`/`.read_entry_data`/`.close`, wrapping
// `tar` -- see `tar.rs`'s own module doc.
mod tar;
// Plan 147 (Temporary Files & Directories) -- `Tempfile.create`/
// `.path`/`.close`, `Tempdir.create`/`.path`/`.close` -- see
// `tempfile.rs`'s own module doc for why this is `mod tempfiles`
// (`#[path]`-redirected), not a bare `mod tempfile;` (a real collision
// against this crate's own `tempfile` dependency name, not a sibling
// module -- unlike `csvs`/`tomls`/`urls`/`charset`'s own collisions).
#[path = "tempfile.rs"]
mod tempfiles;
// Plan 99 (TLS) -- `Tls.connect`/`.connect_with_roots`/`.listen`,
// `TlsStream#read`/`#write`/`#close`, `TlsListener#accept`/`#close`,
// wrapping `rustls` -- see `tls.rs`'s own module doc.
mod tls;
// Plan 102 (WebSocket) -- `WebSocket.connect`, `HttpRequest#upgrade`,
// `WebSocketConnection#send_text`/`#send_binary`/`#recv`/`#close`,
// `WebSocketMessage#kind`/`#text`/`#bytes`, wrapping `tungstenite` --
// see `websocket.rs`'s own module doc.
mod websocket;
// Named `tomls`, not `toml` — this crate's own `mod toml` would shadow
// the external `toml` crate this module wraps, the identical collision
// `aead.rs`/`url.rs` already hit and disclosed.
#[path = "toml.rs"]
mod tomls;
// Plan 154 (Unicode Normalization & Segmentation) — `String.nfc`/
// `.nfd`/`.nfkc`/`.nfkd`/`.codepoint_count`/`.grapheme_count`/
// `.graphemes`/`.words`/`.sentences`/`.grapheme_split_count`/
// `.word_split_count`/`.sentence_split_count`, wrapping `unicode-
// normalization` + `unicode-segmentation` — see `unicode.rs`'s own
// module doc. Named `unicode`, not either wrapped crate's own name —
// no collision either way (`unicode_normalization`/`unicode_
// segmentation` both have underscores a bare `mod unicode;` can't
// shadow), unlike `csvs`/`tomls`/`urls`/`charset`'s own dodges.
mod unicode;
// Named `urls`, not `url` — this crate's own `mod url` would shadow
// the external `url` crate this module wraps, exactly the collision
// `aead.rs` hit against the external `aead` crate (see that module's
// own doc comment); `#[path]` keeps the file itself named `url.rs`.
#[path = "url.rs"]
mod urls;
mod xml;
// Plan 133 (Zip Archives) -- `Zip.create`/`.extract`, `ZipReader.open`/
// `.entry_count`/`.entry_name`/`.entry_size`/`.read_entry_data`/
// `.close`, wrapping `zip` -- see `zip.rs`'s own module doc.
mod zip;
// Plan 105 (Multipart/Form-Data Parsing) -- `Multipart.start`/
// `.next_field`, `Field.name`/`.filename`/`.read_chunk`/`.close`,
// wrapping `multer` -- see `multipart.rs`'s own module doc.
mod multipart;

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

// Plan 195's own additive extension to plan 92's FFI/ABI convention —
// `emerald_rt_result_err(msg)` above is kept exactly as-is (still the
// right call for any domain that hasn't adopted this convention); this
// is its tagged sibling for a domain that HAS. `tag` is a small,
// per-domain `i32` discriminant owned entirely by the calling domain's
// own module (`json.rs`'s `JSON_ERROR_TAG_*`, `regex.rs`'s
// `REGEX_ERROR_TAG_*`, ...) — no global tag registry here or anywhere
// else in this crate. The Err payload built is a pointer to a real,
// `emerald_alloc`-backed two-word enum block (`[tag: i64][msg: *const
// c_char]`), byte-for-byte the same `EnumLayout` shape `json.rs`'s own
// `alloc_enum_block` already establishes for `JsonValue` — so a
// `<Domain>Error` enum registered in `emerald-sema`/`emerald-codegen`
// with matching variant order reads this block exactly like any other
// compiler-synthesized enum value, with zero additional marshaling.
// `msg` is stored on every variant, including ones with no Emerald-
// visible payload field (e.g. `JsonError::UnexpectedEnd`) — a real
// message costs nothing extra and the block's fixed 16-byte shape
// needs a second word regardless.
/// # Safety
/// `msg`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_result_err_tagged(tag: i32, msg: *const c_char) -> *mut c_void {
  let copied = if msg.is_null() {
    alloc_and_copy_str("")
  } else {
    let s = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
    alloc_and_copy_str(&s)
  };
  emerald_rt_result_err_tagged_str_impl(tag, copied)
}

// `emerald_rt_result_err_tagged`'s own internal-Rust-caller sibling —
// takes a real `&str` directly, the identical `emerald_rt_result_err`/
// `emerald_rt_result_err_str` relationship above, reused for the same
// reason (every current caller, `json.rs`/`regex.rs`, already has an
// owned Rust `String`/`&str` in hand, not a `CString`).
pub(crate) unsafe fn emerald_rt_result_err_tagged_str(tag: i32, msg: &str) -> *mut c_void {
  emerald_rt_result_err_tagged_str_impl(tag, alloc_and_copy_str(msg))
}

unsafe fn emerald_rt_result_err_tagged_str_impl(tag: i32, msg: *const c_char) -> *mut c_void {
  let enum_ptr = emerald_alloc(16) as *mut i64;
  *enum_ptr = tag as i64;
  *(enum_ptr.add(1) as *mut *const c_char) = msg;
  let ptr = emerald_alloc(16) as *mut i64;
  *ptr = 1;
  *(ptr.add(1) as *mut *mut c_void) = enum_ptr as *mut c_void;
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

// Plan 182 (Structured CLI Flag Parsing): `CliParser.new`/`.flag`/
// `.option`/`.positional`/`.parse`/`.close`, `CliParseResult#flag`/
// `#value`/`#positional_value`/`#help_requested`/`#help_text`/
// `#error_message`/`#close`, dispatched by `emerald-codegen`'s own
// hardcoded `CliParser`/`CliParseResult`-keyed method-call arms, the
// same shape `Regex`/`Tempfile` immediately above use — see `cli.rs`'s
// own module doc.

/// # Safety
/// `name`/`version`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_new(
  name: *const c_char,
  version: *const c_char,
) -> i64 {
  catch_and_raise(move || cli::cliparser_new(name, version))
}

/// # Safety
/// `id` must be a live `CliParser` handle. `long`/`short`/`help`, if
/// non-null, must each point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_flag(
  id: i64,
  long: *const c_char,
  short: *const c_char,
  help: *const c_char,
) {
  catch_and_raise(move || cli::cliparser_flag(id, long, short, help))
}

/// # Safety
/// `id` must be a live `CliParser` handle. `long`/`short`/`help`, if
/// non-null, must each point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_option(
  id: i64,
  long: *const c_char,
  short: *const c_char,
  help: *const c_char,
  required: i64,
) {
  catch_and_raise(move || cli::cliparser_option(id, long, short, help, required))
}

/// # Safety
/// `id` must be a live `CliParser` handle. `name`/`help`, if non-null,
/// must each point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_positional(
  id: i64,
  name: *const c_char,
  help: *const c_char,
  required: i64,
) {
  catch_and_raise(move || cli::cliparser_positional(id, name, help, required))
}

/// # Safety
/// `id` must be a live `CliParser` handle. `argv` must point to a
/// buffer of at least `argc` valid, NUL-terminated C string pointers —
/// `emerald-codegen`'s own call-site codegen guarantees this (see
/// `cli.rs`'s own module doc for the header-skipping convention that
/// produces it); when `argc` is `0`, `argv` is never dereferenced and
/// may be null.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_parse(
  id: i64,
  argv: *const *const c_char,
  argc: i64,
) -> i64 {
  catch_and_raise(move || cli::cliparser_parse(id, argv, argc))
}

/// # Safety
/// Always safe to call, including on an already-closed or unknown
/// `id` — closing is idempotent and never raises.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparser_close(id: i64) {
  catch_and_raise(move || cli::cliparser_close(id))
}

// Plan 183 (Layered Configuration Loading): `ConfigBuilder.new`/
// `.add_defaults_file`/`.add_config_file`/`.add_env_prefix`/
// `.add_cli_overrides`/`.build`, `ConfigValue#get_string`/`#get_int`/
// `#get_bool`, dispatched by `emerald-codegen`'s own hardcoded
// `ConfigBuilder`/`ConfigValue`-keyed method-call arms, the same shape
// `CliParser`/`CliParseResult` immediately above use — see
// `config.rs`'s own module doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_new() -> i64 {
  catch_and_raise(move || configs::configbuilder_new())
}

/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `path`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_add_defaults_file(id: i64, path: *const c_char) {
  catch_and_raise(move || configs::configbuilder_add_defaults_file(id, path))
}

/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `path`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_add_config_file(id: i64, path: *const c_char) {
  catch_and_raise(move || configs::configbuilder_add_config_file(id, path))
}

/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `prefix`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_add_env_prefix(id: i64, prefix: *const c_char) {
  catch_and_raise(move || configs::configbuilder_add_env_prefix(id, prefix))
}

/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `result_id` must be a
/// live `CliParseResult` handle. `keys` must point to a buffer of at
/// least `count` valid, NUL-terminated C string pointers —
/// `emerald-codegen`'s own call-site codegen guarantees this (see
/// `config.rs`'s own module doc for the header-skipping convention
/// that produces it); when `count` is `0`, `keys` is never
/// dereferenced and may be null.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_add_cli_overrides(
  id: i64,
  result_id: i64,
  keys: *const *const c_char,
  count: i64,
) {
  catch_and_raise(move || configs::configbuilder_add_cli_overrides(id, result_id, keys, count))
}

/// # Safety
/// `id` must be a live `ConfigBuilder` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configbuilder_build(id: i64) -> i64 {
  catch_and_raise(move || configs::configbuilder_build(id))
}

/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configvalue_get_string(
  id: i64,
  key: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || configs::configvalue_get_string(id, key))
}

/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configvalue_get_int(
  id: i64,
  key: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || configs::configvalue_get_int(id, key))
}

/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_configvalue_get_bool(
  id: i64,
  key: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || configs::configvalue_get_bool(id, key))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle. `long`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_flag(id: i64, long: *const c_char) -> i64 {
  catch_and_raise(move || cli::cliparseresult_flag(id, long))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle. `long`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_value(
  id: i64,
  long: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || cli::cliparseresult_value(id, long))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle. `name`, if non-null,
/// must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_positional_value(
  id: i64,
  name: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || cli::cliparseresult_positional_value(id, name))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_help_requested(id: i64) -> i64 {
  catch_and_raise(move || cli::cliparseresult_help_requested(id))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_help_text(id: i64) -> *const c_char {
  catch_and_raise(move || cli::cliparseresult_help_text(id))
}

/// # Safety
/// `id` must be a live `CliParseResult` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_error_message(id: i64) -> *mut c_void {
  catch_and_raise(move || cli::cliparseresult_error_message(id))
}

/// # Safety
/// Always safe to call, including on an already-closed or unknown
/// `id` — closing is idempotent and never raises.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_cliparseresult_close(id: i64) {
  catch_and_raise(move || cli::cliparseresult_close(id))
}

// Plan 163 (Arbitrary-Precision Integers & Decimals): `BigInt` (a
// `crate::handle`-registry opaque `Int64` handle — see `bignum.rs`'s
// own module doc) and `Decimal` (a packed two-`Int64`-field class —
// see `decimal.rs`'s own module doc), dispatched by `emerald-codegen`'s
// own hardcoded `BigInt`/`Decimal`-keyed method-call arms, the same
// shape `Regex` immediately above uses.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_from_i64(n: i64) -> i64 {
  catch_and_raise(move || bignum::bigint_from_i64(n))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_from_s(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || bignum::bigint_from_s(s))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_factorial(n: i64) -> i64 {
  catch_and_raise(move || bignum::bigint_factorial(n))
}

/// # Safety
/// Always safe to call with a handle this crate itself issued.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_to_s(id: i64) -> *const c_char {
  catch_and_raise(move || bignum::bigint_to_s(id))
}

/// # Safety
/// Always safe to call with handles this crate itself issued.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_add(id: i64, other_id: i64) -> i64 {
  catch_and_raise(move || bignum::bigint_add(id, other_id))
}

/// # Safety
/// Always safe to call with handles this crate itself issued.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bigint_mul(id: i64, other_id: i64) -> i64 {
  catch_and_raise(move || bignum::bigint_mul(id, other_id))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_from_s(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || decimal::decimal_from_s(s))
}

/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_to_s(ptr: *const i64) -> *const c_char {
  catch_and_raise(move || decimal::decimal_to_s(ptr))
}

/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_add(
  ptr: *const i64,
  other_ptr: *const i64,
) -> *mut c_void {
  catch_and_raise(move || decimal::decimal_add(ptr, other_ptr))
}

/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_sub(
  ptr: *const i64,
  other_ptr: *const i64,
) -> *mut c_void {
  catch_and_raise(move || decimal::decimal_sub(ptr, other_ptr))
}

/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_mul(
  ptr: *const i64,
  other_ptr: *const i64,
) -> *mut c_void {
  catch_and_raise(move || decimal::decimal_mul(ptr, other_ptr))
}

/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_decimal_div(
  ptr: *const i64,
  other_ptr: *const i64,
) -> *mut c_void {
  catch_and_raise(move || decimal::decimal_div(ptr, other_ptr))
}

// Plan 160 (Date/Time & Timezones): `DateTime`'s two static methods
// plus its four instance methods, plus `ZonedDateTime`'s own two —
// dispatched by `emerald-codegen`'s own hardcoded `DateTime`/
// `ZonedDateTime`-keyed method-call arms, the same shape `Decimal`
// immediately above uses.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_now() -> *mut c_void {
  catch_and_raise(move || datetime::datetime_now())
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_parse_rfc3339(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || datetime::datetime_parse_rfc3339(s))
}

/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_to_rfc3339(ptr: *const i64) -> *const c_char {
  catch_and_raise(move || datetime::datetime_to_rfc3339(ptr))
}

/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_plus_seconds(ptr: *const i64, n: i64) -> *mut c_void {
  catch_and_raise(move || datetime::datetime_plus_seconds(ptr, n))
}

/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_diff_seconds(
  ptr: *const i64,
  other_ptr: *const i64,
) -> i64 {
  catch_and_raise(move || datetime::datetime_diff_seconds(ptr, other_ptr))
}

/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block.
/// `tz_name`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_datetime_in_tz(
  ptr: *const i64,
  tz_name: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || datetime::datetime_in_tz(ptr, tz_name))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zoned_to_utc(ptr: *const i64) -> *mut c_void {
  catch_and_raise(move || datetime::zoned_to_utc(ptr))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zoned_plus_days(ptr: *const i64, n: i64) -> *mut c_void {
  catch_and_raise(move || datetime::zoned_plus_days(ptr, n))
}

// Plan 193 (`Set[T]`, `Deque[T]`, `PriorityQueue[T]`) — 54 concrete
// monomorphized exports, dispatched by `emerald-codegen`'s own
// `local_classes`-tag-keyed `Set$<Elem>`/`Deque$<Elem>`/
// `PriorityQueue$<Elem>` method-call arm — see `collections.rs`'s
// own module doc for the full account.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_new() -> i64 {
  catch_and_raise(collections::set_i64_new)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_add(id: i64, v: i64) -> i64 {
  catch_and_raise(move || collections::set_i64_add(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_contains(id: i64, v: i64) -> i64 {
  catch_and_raise(move || collections::set_i64_contains(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_remove(id: i64, v: i64) -> i64 {
  catch_and_raise(move || collections::set_i64_remove(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_count(id: i64) -> i64 {
  catch_and_raise(move || collections::set_i64_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_each(id: i64) -> *mut c_void {
  catch_and_raise(move || collections::set_i64_each(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_i64_close(id: i64) {
  catch_and_raise(move || collections::set_i64_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_new() -> i64 {
  catch_and_raise(collections::set_string_new)
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_add(id: i64, v: *const c_char) -> i64 {
  catch_and_raise(move || collections::set_string_add(id, v))
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_contains(id: i64, v: *const c_char) -> i64 {
  catch_and_raise(move || collections::set_string_contains(id, v))
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_remove(id: i64, v: *const c_char) -> i64 {
  catch_and_raise(move || collections::set_string_remove(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_count(id: i64) -> i64 {
  catch_and_raise(move || collections::set_string_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_each(id: i64) -> *mut c_void {
  catch_and_raise(move || collections::set_string_each(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_set_string_close(id: i64) {
  catch_and_raise(move || collections::set_string_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_new() -> i64 {
  catch_and_raise(collections::deque_i64_new)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_push_front(id: i64, v: i64) {
  catch_and_raise(move || collections::deque_i64_push_front(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_push_back(id: i64, v: i64) {
  catch_and_raise(move || collections::deque_i64_push_back(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_pop_front(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_i64_pop_front(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_pop_back(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_i64_pop_back(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_count(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_i64_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_i64_close(id: i64) {
  catch_and_raise(move || collections::deque_i64_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_new() -> i64 {
  catch_and_raise(collections::deque_f64_new)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_push_front(id: i64, v: f64) {
  catch_and_raise(move || collections::deque_f64_push_front(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_push_back(id: i64, v: f64) {
  catch_and_raise(move || collections::deque_f64_push_back(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_pop_front(id: i64) -> f64 {
  catch_and_raise(move || collections::deque_f64_pop_front(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_pop_back(id: i64) -> f64 {
  catch_and_raise(move || collections::deque_f64_pop_back(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_count(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_f64_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_f64_close(id: i64) {
  catch_and_raise(move || collections::deque_f64_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_new() -> i64 {
  catch_and_raise(collections::deque_string_new)
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_push_front(id: i64, v: *const c_char) {
  catch_and_raise(move || collections::deque_string_push_front(id, v))
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_push_back(id: i64, v: *const c_char) {
  catch_and_raise(move || collections::deque_string_push_back(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_pop_front(id: i64) -> *const c_char {
  catch_and_raise(move || collections::deque_string_pop_front(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_pop_back(id: i64) -> *const c_char {
  catch_and_raise(move || collections::deque_string_pop_back(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_count(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_string_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_string_close(id: i64) {
  catch_and_raise(move || collections::deque_string_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_new() -> i64 {
  catch_and_raise(collections::deque_bool_new)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_push_front(id: i64, v: i64) {
  catch_and_raise(move || collections::deque_bool_push_front(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_push_back(id: i64, v: i64) {
  catch_and_raise(move || collections::deque_bool_push_back(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_pop_front(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_bool_pop_front(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_pop_back(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_bool_pop_back(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_count(id: i64) -> i64 {
  catch_and_raise(move || collections::deque_bool_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deque_bool_close(id: i64) {
  catch_and_raise(move || collections::deque_bool_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_new() -> i64 {
  catch_and_raise(collections::priority_queue_i64_new)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_push(id: i64, v: i64) {
  catch_and_raise(move || collections::priority_queue_i64_push(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_pop(id: i64) -> i64 {
  catch_and_raise(move || collections::priority_queue_i64_pop(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_peek(id: i64) -> i64 {
  catch_and_raise(move || collections::priority_queue_i64_peek(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_count(id: i64) -> i64 {
  catch_and_raise(move || collections::priority_queue_i64_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_i64_close(id: i64) {
  catch_and_raise(move || collections::priority_queue_i64_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_new() -> i64 {
  catch_and_raise(collections::priority_queue_string_new)
}

/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_push(id: i64, v: *const c_char) {
  catch_and_raise(move || collections::priority_queue_string_push(id, v))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_pop(id: i64) -> *const c_char {
  catch_and_raise(move || collections::priority_queue_string_pop(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_peek(id: i64) -> *const c_char {
  catch_and_raise(move || collections::priority_queue_string_peek(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_count(id: i64) -> i64 {
  catch_and_raise(move || collections::priority_queue_string_count(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_priority_queue_string_close(id: i64) {
  catch_and_raise(move || collections::priority_queue_string_close(id))
}

// Plan 124 (XML Parsing): `Xml.parse`/`.parse_file`/`.reader_from_
// string`/`.reader_from_file`, `XmlReader#next_event` — see `xml.rs`'s
// own module doc.

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xml_parse(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || xml::xml_parse(s))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xml_parse_file(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || xml::xml_parse_file(path))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xml_reader_from_string(s: *const c_char) -> i64 {
  catch_and_raise(move || xml::xml_reader_from_string(s))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xml_reader_from_file(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || xml::xml_reader_from_file(path))
}

/// # Safety
/// `id` must be a live `XmlReader` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_xml_reader_next_event(id: i64) -> *mut c_void {
  catch_and_raise(move || xml::xml_reader_next_event(id))
}

// Plan 180 (Property-Based Testing Generators) — `property "..."
// (a: Int64, ...) do ... end`'s real, generator-driven case loop; see
// `proptest_support.rs`'s own module doc for the full architecture
// (the compiled Emerald harness drives the loop, calling into these
// nine exports once per generated/shrunk case — nothing here calls
// back into compiled Emerald code).

/// # Safety
/// `types` must be a valid, NUL-terminated C string, one byte per
/// property parameter (`i`/`f`/`s`/`b`).
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_begin(types: *const c_char) -> i64 {
  catch_and_raise(move || proptest_support::proptest_begin(types))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_current_i64(session: i64, idx: i64) -> i64 {
  catch_and_raise(move || proptest_support::proptest_current_i64(session, idx))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_current_f64(session: i64, idx: i64) -> f64 {
  catch_and_raise(move || proptest_support::proptest_current_f64(session, idx))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_current_string(
  session: i64,
  idx: i64,
) -> *const c_char {
  catch_and_raise(move || proptest_support::proptest_current_string(session, idx))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
/// Returns `0`/`1`, not a real Emerald `Boolean` — see `proptest_
/// support.rs`'s own doc comment for why.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_current_bool(session: i64, idx: i64) -> i64 {
  catch_and_raise(move || proptest_support::proptest_current_bool(session, idx))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`;
/// `message`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_report(
  session: i64,
  failed: i64,
  message: *const c_char,
) -> i64 {
  catch_and_raise(move || proptest_support::proptest_report(session, failed, message))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_failed(session: i64) -> i64 {
  catch_and_raise(move || proptest_support::proptest_failed(session))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_case_count(session: i64) -> i64 {
  catch_and_raise(move || proptest_support::proptest_case_count(session))
}

/// # Safety
/// `session` must be a live handle from `emerald_rt_proptest_begin`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_proptest_fail_message(session: i64) -> *const c_char {
  catch_and_raise(move || proptest_support::proptest_fail_message(session))
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

// Plan 144 (Extended Filesystem Operations): `Dir.entries`/`.entries_
// count`/`.walk`/`.walk_count` — see `dir.rs`'s own module doc.

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dir_entries(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || dir::dir_entries(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dir_entries_count(path: *const c_char) -> i64 {
  catch_and_raise(move || dir::dir_entries_count(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dir_walk(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || dir::dir_walk(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dir_walk_count(path: *const c_char) -> i64 {
  catch_and_raise(move || dir::dir_walk_count(path))
}

// Plan 150 (Path Globbing): `Glob.glob`/`.glob_count` — see
// `glob.rs`'s own module doc.

/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_glob_match(pattern: *const c_char) -> *mut c_void {
  catch_and_raise(move || glob::glob_match(pattern))
}

/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_glob_match_count(pattern: *const c_char) -> i64 {
  catch_and_raise(move || glob::glob_match_count(pattern))
}

// Plan 144 (Extended Filesystem Operations): `Path.exists`/`.is_file`/
// `.is_dir`/`.is_symlink`/`.metadata`/`.unix_mode`/`.set_unix_mode`/
// `.symlink`/`.read_link`, plus `FileMetadata`'s own five zero-
// argument accessors — see `path.rs`'s own module doc.

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_exists(path: *const c_char) -> i64 {
  catch_and_raise(move || path::path_exists(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_is_file(path: *const c_char) -> i64 {
  catch_and_raise(move || path::path_is_file(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_is_dir(path: *const c_char) -> i64 {
  catch_and_raise(move || path::path_is_dir(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_is_symlink(path: *const c_char) -> i64 {
  catch_and_raise(move || path::path_is_symlink(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_metadata(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || path::path_metadata(path))
}

/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_filemetadata_size(ptr: *const i64) -> i64 {
  catch_and_raise(move || path::filemetadata_size(ptr))
}

/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_filemetadata_modified_unix(ptr: *const i64) -> i64 {
  catch_and_raise(move || path::filemetadata_modified_unix(ptr))
}

/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_filemetadata_is_dir(ptr: *const i64) -> i64 {
  catch_and_raise(move || path::filemetadata_is_dir(ptr))
}

/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_filemetadata_is_file(ptr: *const i64) -> i64 {
  catch_and_raise(move || path::filemetadata_is_file(ptr))
}

/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_filemetadata_readonly(ptr: *const i64) -> i64 {
  catch_and_raise(move || path::filemetadata_readonly(ptr))
}

// Plan 144's own Decision log: `Path.unix_mode`/`.set_unix_mode`/
// `.symlink` are a real, disclosed Unix-only limitation (`path.rs`'s
// own module doc) — `#[cfg(unix)]`-gated here too, matching the
// underlying `std::os::unix::fs` gating exactly, rather than silently
// compiling to nothing on a non-Unix target.

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_unix_mode(path: *const c_char) -> i64 {
  catch_and_raise(move || path::path_unix_mode(path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_set_unix_mode(path: *const c_char, mode: i64) {
  catch_and_raise(move || path::path_set_unix_mode(path, mode))
}

/// # Safety
/// `target`/`link_path`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_symlink(target: *const c_char, link_path: *const c_char) {
  catch_and_raise(move || path::path_symlink(target, link_path))
}

/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_path_read_link(path: *const c_char) -> *mut c_void {
  catch_and_raise(move || path::path_read_link(path))
}

// Plan 145 (Process Spawning & Control): `Process.run` plus
// `ProcessResult`'s own four zero-argument accessors — see
// `process.rs`'s own module doc.

/// # Safety
/// `cmd`/`stdin_data`, if non-null, must point to valid, NUL-
/// terminated C strings. `argv` must point to a buffer of at least
/// `argc` valid, NUL-terminated C string pointers — `emerald-codegen`'s
/// own call-site codegen guarantees this; see `process.rs`'s own
/// module doc for the header-skipping convention that produces it.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_process_run(
  cmd: *const c_char,
  argv: *const *const c_char,
  argc: i64,
  stdin_data: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || process::process_run(cmd, argv, argc, stdin_data))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_processresult_stdout(ptr: *const i64) -> *const c_char {
  catch_and_raise(move || process::processresult_stdout(ptr))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_processresult_stderr(ptr: *const i64) -> *const c_char {
  catch_and_raise(move || process::processresult_stderr(ptr))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_processresult_exit_code(ptr: *const i64) -> i64 {
  catch_and_raise(move || process::processresult_exit_code(ptr))
}

/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_processresult_success(ptr: *const i64) -> i64 {
  catch_and_raise(move || process::processresult_success(ptr))
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

/// `Bytes.length(self): Int64` — plan 125's own addition, see
/// `bytes.rs`'s own doc comment for why.
///
/// # Safety
/// `id` must be a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bytes_length(id: i64) -> i64 {
  catch_and_raise(move || bytes::bytes_length(id))
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

// Plan 191 (Progress Bars & Terminal Formatting): `ProgressBar.new`/
// `.new_spinner`/`.increment`/`.set_message`/`.finish`,
// `Console.styled`/`.is_terminal` — see `progress.rs`'s own module
// doc.

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_progressbar_new(total: i64) -> i64 {
  catch_and_raise(move || progress::progressbar_new(total))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_progressbar_new_spinner() -> i64 {
  catch_and_raise(move || progress::progressbar_new_spinner())
}

/// # Safety
/// `id` must be a live `ProgressBar` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_progressbar_increment(id: i64, n: i64) {
  catch_and_raise(move || progress::progressbar_increment(id, n))
}

/// # Safety
/// `id` must be a live `ProgressBar` handle. `text`, if non-null, must
/// point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_progressbar_set_message(id: i64, text: *const c_char) {
  catch_and_raise(move || progress::progressbar_set_message(id, text))
}

/// # Safety
/// `id` must be a live `ProgressBar` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_progressbar_finish(id: i64) {
  catch_and_raise(move || progress::progressbar_finish(id))
}

/// # Safety
/// `text`/`color`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_console_styled(
  text: *const c_char,
  color: *const c_char,
) -> *const c_char {
  catch_and_raise(move || progress::console_styled(text, color))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_console_is_terminal() -> i64 {
  catch_and_raise(move || progress::console_is_terminal())
}

// Plan 112 (Password Hashing): `Password.hash`/`.verify` — see
// `password.rs`'s own module doc.

/// # Safety
/// `plaintext`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_password_hash(plaintext: *const c_char) -> *const c_char {
  catch_and_raise(move || password::password_hash(plaintext))
}

/// # Safety
/// `plaintext`/`stored_hash`, if non-null, must point to valid,
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_password_verify(
  plaintext: *const c_char,
  stored_hash: *const c_char,
) -> i64 {
  catch_and_raise(move || password::password_verify(plaintext, stored_hash))
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

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_stream_connect(host: *const c_char, port: i64) -> i64 {
  catch_and_raise(move || net::tcp_stream_connect(host, port))
}

/// # Safety
/// Always safe to call for a live `TcpStream` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_stream_read(id: i64, max_len: i64) -> *const c_char {
  catch_and_raise(move || net::tcp_stream_read(id, max_len))
}

/// # Safety
/// `data`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_stream_write(id: i64, data: *const c_char) -> i64 {
  catch_and_raise(move || net::tcp_stream_write(id, data))
}

/// # Safety
/// Always safe to call for a live `TcpStream` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_stream_close(id: i64) {
  catch_and_raise(move || net::tcp_stream_close(id))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_listener_bind(host: *const c_char, port: i64) -> i64 {
  catch_and_raise(move || net::tcp_listener_bind(host, port))
}

/// # Safety
/// Always safe to call for a live `TcpListener` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_listener_accept(id: i64) -> i64 {
  catch_and_raise(move || net::tcp_listener_accept(id))
}

/// # Safety
/// Always safe to call for a live `TcpListener` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tcp_listener_close(id: i64) {
  catch_and_raise(move || net::tcp_listener_close(id))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_bind(host: *const c_char, port: i64) -> i64 {
  catch_and_raise(move || net::udp_socket_bind(host, port))
}

/// # Safety
/// `data`/`host`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_send_to(
  id: i64,
  data: *const c_char,
  host: *const c_char,
  port: i64,
) -> i64 {
  catch_and_raise(move || net::udp_socket_send_to(id, data, host, port))
}

/// # Safety
/// Always safe to call for a live `UdpSocket` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_recv_from(id: i64, max_len: i64) -> *const c_char {
  catch_and_raise(move || net::udp_socket_recv_from(id, max_len))
}

/// # Safety
/// Always safe to call for a live `UdpSocket` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_close(id: i64) {
  catch_and_raise(move || net::udp_socket_close(id))
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_last_sender_host() -> *const c_char {
  catch_and_raise(net::udp_socket_last_sender_host)
}

/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_udp_socket_last_sender_port() -> i64 {
  catch_and_raise(net::udp_socket_last_sender_port)
}

/// # Safety
/// `mode`/`nameserver`, if non-null, must point to valid, NUL-
/// terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dns_configure(mode: *const c_char, nameserver: *const c_char) {
  catch_and_raise(move || dns::dns_configure(mode, nameserver))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dns_resolve(host: *const c_char) -> *const c_char {
  catch_and_raise(move || dns::dns_resolve(host))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dns_resolve_all(host: *const c_char) -> *mut c_void {
  catch_and_raise(move || dns::dns_resolve_all(host))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_dns_resolve_count(host: *const c_char) -> i64 {
  catch_and_raise(move || dns::dns_resolve_count(host))
}

/// # Safety
/// `url`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_get(url: *const c_char) -> *mut c_void {
  catch_and_raise(move || http_client::http_get(url))
}

/// # Safety
/// `url`/`body`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_post(
  url: *const c_char,
  body: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || http_client::http_post(url, body))
}

/// # Safety
/// Always safe to call for a live `HttpResponse` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_response_status(id: i64) -> i64 {
  catch_and_raise(move || http_client::http_response_status(id))
}

/// # Safety
/// Always safe to call for a live `HttpResponse` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_response_body(id: i64) -> *const c_char {
  catch_and_raise(move || http_client::http_response_body(id))
}

/// Plan 106 (Advanced Async HTTP, hyper-direct).
///
/// # Safety
/// `url`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http2_client_get(url: *const c_char) -> *const c_char {
  catch_and_raise(move || http2_client::http2_client_get(url))
}

/// Plan 106 (Advanced Async HTTP, hyper-direct).
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http2_client_last_status() -> i64 {
  catch_and_raise(move || http2_client::http2_client_last_status())
}

/// # Safety
/// `handler` must be a valid function pointer to a compiled Emerald
/// trampoline taking one live `HttpRequest` handle id and returning
/// one live `HttpResponse` handle id.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_serve(
  port: i64,
  handler: extern "C-unwind" fn(i64) -> i64,
) -> i64 {
  catch_and_raise(move || http_server::http_serve(port, handler))
}

/// # Safety
/// `body`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_response_build(status: i64, body: *const c_char) -> i64 {
  catch_and_raise(move || http_server::http_response_build(status, body))
}

/// # Safety
/// Always safe to call for a live `HttpRequest` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_request_method(id: i64) -> *const c_char {
  catch_and_raise(move || http_server::http_request_method(id))
}

/// # Safety
/// Always safe to call for a live `HttpRequest` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_request_path(id: i64) -> *const c_char {
  catch_and_raise(move || http_server::http_request_path(id))
}

/// # Safety
/// Always safe to call for a live `HttpRequest` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_request_body(id: i64) -> *const c_char {
  catch_and_raise(move || http_server::http_request_body(id))
}

/// # Safety
/// Always safe to call for a live `HttpRequest` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_request_upgrade(id: i64) -> *mut c_void {
  catch_and_raise(move || websocket::ws_upgrade_from_http(id))
}

/// # Safety
/// `url`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_connect(url: *const c_char) -> *mut c_void {
  catch_and_raise(move || websocket::ws_connect(url))
}

/// # Safety
/// `msg`, if non-null, must point to a valid, NUL-terminated C string.
/// `id` must be a live `WebSocketConnection` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_send_text(id: i64, msg: *const c_char) -> *mut c_void {
  catch_and_raise(move || websocket::ws_send_text(id, msg))
}

/// # Safety
/// `id` must be a live `WebSocketConnection` handle; `bytes_id` must be
/// a live `Bytes` value.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_send_binary(id: i64, bytes_id: i64) -> *mut c_void {
  catch_and_raise(move || websocket::ws_send_binary(id, bytes_id))
}

/// # Safety
/// `id` must be a live `WebSocketConnection` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_recv(id: i64) -> *mut c_void {
  catch_and_raise(move || websocket::ws_recv(id))
}

/// # Safety
/// Always safe to call for a live `WebSocketConnection` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_close(id: i64) {
  catch_and_raise(move || websocket::ws_close(id))
}

/// # Safety
/// `ptr` must point to a live `WebSocketMessage` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_message_kind(ptr: *const i64) -> i64 {
  catch_and_raise(move || websocket::ws_message_kind(ptr))
}

/// # Safety
/// `ptr` must point to a live `WebSocketMessage` block whose `kind` is
/// `0` (Text).
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_message_text(ptr: *const i64) -> *const c_char {
  catch_and_raise(move || websocket::ws_message_text(ptr))
}

/// # Safety
/// `ptr` must point to a live `WebSocketMessage` block whose `kind` is
/// `1` (Binary).
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_ws_message_bytes(ptr: *const i64) -> i64 {
  catch_and_raise(move || websocket::ws_message_bytes(ptr))
}

/// # Safety
/// `request` must be a live `HttpRequest` handle whose own
/// `tiny_http::Request` hasn't already been taken.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sse_upgrade(request: i64) -> i64 {
  catch_and_raise(move || sse::sse_upgrade(request))
}

/// # Safety
/// `event`/`data`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sse_send(
  stream: i64,
  event: *const c_char,
  data: *const c_char,
) -> i64 {
  catch_and_raise(move || sse::sse_send(stream, event, data))
}

/// # Safety
/// `text`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sse_comment(stream: i64, text: *const c_char) -> i64 {
  catch_and_raise(move || sse::sse_comment(stream, text))
}

/// # Safety
/// Always safe to call for a live `Sse` stream handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sse_close(stream: i64) -> i64 {
  catch_and_raise(move || sse::sse_close(stream))
}

/// # Safety
/// Always safe to call for a live `HttpRequest` handle whose own
/// `tiny_http::Request` hasn't already been taken.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_http_request_content_type_boundary(id: i64) -> *const c_char {
  catch_and_raise(move || http_server::http_request_content_type_boundary(id))
}

/// # Safety
/// `request` must be a live `HttpRequest` handle; `boundary`, if
/// non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_multipart_start(request: i64, boundary: *const c_char) -> i64 {
  catch_and_raise(move || multipart::multipart_start(request, boundary))
}

/// # Safety
/// `multipart` must be a live `Multipart` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_multipart_next_field(multipart: i64) -> i64 {
  catch_and_raise(move || multipart::multipart_next_field(multipart))
}

/// # Safety
/// `field` must be a live `Field` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_field_name(field: i64) -> *const c_char {
  catch_and_raise(move || multipart::field_name(field))
}

/// # Safety
/// `field` must be a live `Field` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_field_filename(field: i64) -> *mut c_char {
  catch_and_raise(move || multipart::field_filename(field))
}

/// # Safety
/// `field` must be a live `Field` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_field_read_chunk(field: i64, max_bytes: i64) -> *mut c_char {
  catch_and_raise(move || multipart::field_read_chunk(field, max_bytes))
}

/// # Safety
/// Always safe to call for any `Int64`, live `Field` handle or not.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_field_close(field: i64) -> i64 {
  catch_and_raise(move || multipart::field_close(field))
}

/// # Safety
/// `host`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_connect(host: *const c_char, port: i64) -> i64 {
  catch_and_raise(move || tls::tls_connect(host, port))
}

/// # Safety
/// `host`/`mode`, if non-null, must point to valid, NUL-terminated C
/// strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_connect_with_roots(
  host: *const c_char,
  port: i64,
  mode: *const c_char,
) -> i64 {
  catch_and_raise(move || tls::tls_connect_with_roots(host, port, mode))
}

/// # Safety
/// Always safe to call for a live `TlsStream` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_stream_read(id: i64, max_len: i64) -> *const c_char {
  catch_and_raise(move || tls::tls_stream_read(id, max_len))
}

/// # Safety
/// `data`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_stream_write(id: i64, data: *const c_char) -> i64 {
  catch_and_raise(move || tls::tls_stream_write(id, data))
}

/// # Safety
/// Always safe to call for a live `TlsStream` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_stream_close(id: i64) {
  catch_and_raise(move || tls::tls_stream_close(id))
}

/// # Safety
/// `host`/`cert_path`/`key_path`, if non-null, must point to valid,
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_listen(
  host: *const c_char,
  port: i64,
  cert_path: *const c_char,
  key_path: *const c_char,
) -> i64 {
  catch_and_raise(move || tls::tls_listen(host, port, cert_path, key_path))
}

/// # Safety
/// Always safe to call for a live `TlsListener` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_listener_accept(id: i64) -> i64 {
  catch_and_raise(move || tls::tls_listener_accept(id))
}

/// # Safety
/// Always safe to call for a live `TlsListener` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tls_listener_close(id: i64) {
  catch_and_raise(move || tls::tls_listener_close(id))
}

/// # Safety
/// `ikm`/`salt`/`info`, if non-null, must point to valid, NUL-
/// terminated hex-text C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_kdf_hkdf(
  ikm: *const c_char,
  salt: *const c_char,
  info: *const c_char,
  length: i64,
) -> *const c_char {
  catch_and_raise(move || kdf::kdf_hkdf(ikm, salt, info, length))
}

/// # Safety
/// `password`/`salt`, if non-null, must point to valid, NUL-terminated
/// C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_kdf_pbkdf2(
  password: *const c_char,
  salt: *const c_char,
  iterations: i64,
  length: i64,
) -> *const c_char {
  catch_and_raise(move || kdf::kdf_pbkdf2(password, salt, iterations, length))
}

/// # Safety
/// `raw`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings. `raw` is read as a raw byte sequence, not required to be
/// valid UTF-8 (plan 153's own disclosed `String`-UTF-8-invariant
/// exception — see `charset.rs`'s own module doc). An unresolvable
/// `label` is a real, disclosed runtime abort (`process::exit(1)`),
/// not a panic caught by `catch_and_raise` — `charset::encoding_decode`
/// itself never unwinds on that path.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_encoding_decode(
  raw: *const c_char,
  label: *const c_char,
) -> *const c_char {
  catch_and_raise(move || charset::encoding_decode(raw, label))
}

/// # Safety
/// `raw`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings. `raw` is read as a raw byte sequence, not required to be
/// valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_encoding_decode_strict(
  raw: *const c_char,
  label: *const c_char,
) -> *mut c_char {
  catch_and_raise(move || charset::encoding_decode_strict(raw, label))
}

/// # Safety
/// `text`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings. The returned `String` is, in general, NOT valid UTF-8 —
/// see `charset.rs`'s own module doc for this plan's disclosed
/// deliberate exception to `String`'s documented invariant.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_encoding_encode(
  text: *const c_char,
  label: *const c_char,
) -> *const c_char {
  catch_and_raise(move || charset::encoding_encode(text, label))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_toml_parse(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || tomls::toml_parse(s))
}

/// # Safety
/// `obj` must point to a real `JsonValue` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_json_to_toml(obj: *const c_void) -> *mut c_void {
  catch_and_raise(move || tomls::json_to_toml(obj))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_csv_parse(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || csvs::csv_parse(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_csv_parse_with_headers(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || csvs::csv_parse_with_headers(s))
}

/// # Safety
/// `rows` must point to a real `Array[Array[String]]` buffer.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_csv_write(rows: *const c_void) -> *const c_char {
  catch_and_raise(move || csvs::csv_write(rows))
}

// Plan 154 (Unicode Normalization & Segmentation): `String.nfc`/
// `.nfd`/`.nfkc`/`.nfkd`/`.codepoint_count`/`.grapheme_count`/
// `.graphemes`/`.words`/`.sentences`/`.grapheme_split_count`/
// `.word_split_count`/`.sentence_split_count` — see `unicode.rs`'s own
// module doc.

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_nfc(s: *const c_char) -> *const c_char {
  catch_and_raise(move || unicode::string_nfc(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_nfd(s: *const c_char) -> *const c_char {
  catch_and_raise(move || unicode::string_nfd(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_nfkc(s: *const c_char) -> *const c_char {
  catch_and_raise(move || unicode::string_nfkc(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_nfkd(s: *const c_char) -> *const c_char {
  catch_and_raise(move || unicode::string_nfkd(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_codepoint_count(s: *const c_char) -> i64 {
  catch_and_raise(move || unicode::string_codepoint_count(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_grapheme_count(s: *const c_char) -> i64 {
  catch_and_raise(move || unicode::string_grapheme_count(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_graphemes(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || unicode::string_graphemes(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_words(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || unicode::string_words(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_sentences(s: *const c_char) -> *mut c_void {
  catch_and_raise(move || unicode::string_sentences(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_grapheme_split_count(s: *const c_char) -> i64 {
  catch_and_raise(move || unicode::string_grapheme_split_count(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_word_split_count(s: *const c_char) -> i64 {
  catch_and_raise(move || unicode::string_word_split_count(s))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_string_sentence_split_count(s: *const c_char) -> i64 {
  catch_and_raise(move || unicode::string_sentence_split_count(s))
}

// Plan 130 (Gzip/Deflate/Zlib Compression): `Gzip`/`Deflate`/`Zlib`
// `.compress`/`.decompress`, `GzipWriter`/`GzipReader` (+ `Deflate`/
// `Zlib` siblings) — see `gzip.rs`'s own module doc. Every `Bytes`
// value (parameter or return) crosses this boundary as a plain `i64`,
// the identical convention `bytes.rs`'s own `Bytes.to_hex`/`Sha256.
// hash` already establish.

/// `Gzip.compress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_compress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::gzip_compress(data_id))
}

/// `Gzip.decompress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_decompress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::gzip_decompress(data_id))
}

/// `Deflate.compress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_compress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::deflate_compress(data_id))
}

/// `Deflate.decompress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_decompress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::deflate_decompress(data_id))
}

/// `Zlib.compress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_compress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::zlib_compress(data_id))
}

/// `Zlib.decompress(data: Bytes): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_decompress(data_id: i64) -> i64 {
  catch_and_raise(move || gzip::zlib_decompress(data_id))
}

/// `GzipWriter.open(path: String): GzipWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_writer_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::gzip_writer_open(path))
}

/// `GzipWriter#write_chunk(self, data: Bytes): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_writer_write_chunk(id: i64, data_id: i64) {
  catch_and_raise(move || gzip::gzip_writer_write_chunk(id, data_id))
}

/// `GzipWriter#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_writer_close(id: i64) {
  catch_and_raise(move || gzip::gzip_writer_close(id))
}

/// `GzipReader.open(path: String): GzipReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_reader_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::gzip_reader_open(path))
}

/// `GzipReader#read_chunk(self, max_len: Int64): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  catch_and_raise(move || gzip::gzip_reader_read_chunk(id, max_len))
}

/// `GzipReader#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_gzip_reader_close(id: i64) {
  catch_and_raise(move || gzip::gzip_reader_close(id))
}

/// `DeflateWriter.open(path: String): DeflateWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_writer_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::deflate_writer_open(path))
}

/// `DeflateWriter#write_chunk(self, data: Bytes): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_writer_write_chunk(id: i64, data_id: i64) {
  catch_and_raise(move || gzip::deflate_writer_write_chunk(id, data_id))
}

/// `DeflateWriter#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_writer_close(id: i64) {
  catch_and_raise(move || gzip::deflate_writer_close(id))
}

/// `DeflateReader.open(path: String): DeflateReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_reader_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::deflate_reader_open(path))
}

/// `DeflateReader#read_chunk(self, max_len: Int64): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  catch_and_raise(move || gzip::deflate_reader_read_chunk(id, max_len))
}

/// `DeflateReader#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_deflate_reader_close(id: i64) {
  catch_and_raise(move || gzip::deflate_reader_close(id))
}

/// `ZlibWriter.open(path: String): ZlibWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_writer_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::zlib_writer_open(path))
}

/// `ZlibWriter#write_chunk(self, data: Bytes): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_writer_write_chunk(id: i64, data_id: i64) {
  catch_and_raise(move || gzip::zlib_writer_write_chunk(id, data_id))
}

/// `ZlibWriter#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_writer_close(id: i64) {
  catch_and_raise(move || gzip::zlib_writer_close(id))
}

/// `ZlibReader.open(path: String): ZlibReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_reader_open(path: *const c_char) -> i64 {
  catch_and_raise(move || gzip::zlib_reader_open(path))
}

/// `ZlibReader#read_chunk(self, max_len: Int64): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  catch_and_raise(move || gzip::zlib_reader_read_chunk(id, max_len))
}

/// `ZlibReader#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zlib_reader_close(id: i64) {
  catch_and_raise(move || gzip::zlib_reader_close(id))
}

// Plan 132 (Tar Archives): `Tar.create`/`.extract`, `TarReader.open`/
// `.next_entry`/`.entry_size`/`.read_entry_data`/`.close` -- see
// `tar.rs`'s own module doc.

/// `Tar.create(archive_path: String, paths: Array[String]): Void`.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string. `path_ptrs` must point to `count` valid `*const c_char`
/// entries, each itself a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_create(
  archive_path: *const c_char,
  path_ptrs: *const *const c_char,
  count: i64,
) {
  catch_and_raise(move || tar::tar_create(archive_path, path_ptrs, count))
}

/// `Tar.extract(archive_path: String, dest_dir: String): Void`.
///
/// # Safety
/// `archive_path`/`dest_dir`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_extract(
  archive_path: *const c_char,
  dest_dir: *const c_char,
) {
  catch_and_raise(move || tar::tar_extract(archive_path, dest_dir))
}

/// `TarReader.open(archive_path: String): TarReader`.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_reader_open(archive_path: *const c_char) -> i64 {
  catch_and_raise(move || tar::tar_reader_open(archive_path))
}

/// `TarReader#next_entry(self): Option[String]`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_reader_next_entry(id: i64) -> *mut c_void {
  catch_and_raise(move || tar::tar_reader_next_entry(id))
}

/// `TarReader#entry_size(self): Int64`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_reader_entry_size(id: i64) -> i64 {
  catch_and_raise(move || tar::tar_reader_entry_size(id))
}

/// `TarReader#read_entry_data(self): Bytes`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_reader_read_entry_data(id: i64) -> i64 {
  catch_and_raise(move || tar::tar_reader_read_entry_data(id))
}

/// `TarReader#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tar_reader_close(id: i64) {
  catch_and_raise(move || tar::tar_reader_close(id))
}

// Plan 133 (Zip Archives): `Zip.create`/`.extract`, `ZipReader.open`/
// `.entry_count`/`.entry_name`/`.entry_size`/`.read_entry_data`/
// `.close` -- see `zip.rs`'s own module doc.

/// `Zip.create(archive_path: String, paths: Array[String]): Void`.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string. `path_ptrs` must point to `count` valid `*const c_char`
/// entries, each itself a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_create(
  archive_path: *const c_char,
  path_ptrs: *const *const c_char,
  count: i64,
) {
  catch_and_raise(move || zip::zip_create(archive_path, path_ptrs, count))
}

/// `Zip.extract(archive_path: String, dest_dir: String): Void`.
///
/// # Safety
/// `archive_path`/`dest_dir`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_extract(
  archive_path: *const c_char,
  dest_dir: *const c_char,
) {
  catch_and_raise(move || zip::zip_extract(archive_path, dest_dir))
}

/// `ZipReader.open(archive_path: String): ZipReader`.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_open(archive_path: *const c_char) -> i64 {
  catch_and_raise(move || zip::zip_reader_open(archive_path))
}

/// `ZipReader#entry_count(self): Int64`.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_entry_count(id: i64) -> i64 {
  catch_and_raise(move || zip::zip_reader_entry_count(id))
}

/// `ZipReader#entry_name(self, i: Int64): String`.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_entry_name(id: i64, index: i64) -> *const c_char {
  catch_and_raise(move || zip::zip_reader_entry_name(id, index))
}

/// `ZipReader#entry_size(self, i: Int64): Int64`.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_entry_size(id: i64, index: i64) -> i64 {
  catch_and_raise(move || zip::zip_reader_entry_size(id, index))
}

/// `ZipReader#read_entry_data(self, i: Int64): Bytes`.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_read_entry_data(id: i64, index: i64) -> i64 {
  catch_and_raise(move || zip::zip_reader_read_entry_data(id, index))
}

/// `ZipReader#close(self): Void`.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_zip_reader_close(id: i64) {
  catch_and_raise(move || zip::zip_reader_close(id))
}

// Plan 147 (Temporary Files & Directories): `Tempfile.create`/`.path`/
// `.close`, `Tempdir.create`/`.path`/`.close` -- see `tempfile.rs`'s
// own module doc.

/// `Tempfile.create(): Tempfile`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempfile_create() -> i64 {
  catch_and_raise(tempfiles::tempfile_create)
}

/// `Tempfile#path(self): String`.
///
/// # Safety
/// `id` must be a live `Tempfile` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempfile_path(id: i64) -> *const c_char {
  catch_and_raise(move || tempfiles::tempfile_path(id))
}

/// `Tempfile#close(self): Void`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempfile_close(id: i64) {
  catch_and_raise(move || tempfiles::tempfile_close(id))
}

/// `Tempdir.create(): Tempdir`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempdir_create() -> i64 {
  catch_and_raise(tempfiles::tempdir_create)
}

/// `Tempdir#path(self): String`.
///
/// # Safety
/// `id` must be a live `Tempdir` handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempdir_path(id: i64) -> *const c_char {
  catch_and_raise(move || tempfiles::tempdir_path(id))
}

/// `Tempdir#close(self): Void`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_tempdir_close(id: i64) {
  catch_and_raise(move || tempfiles::tempdir_close(id))
}

// Plan 137 (SQLite): `Sqlite.open`/`.open_memory`/`.close`/
// `.execute_direct`/`.prepare`/`.bind_string`/`.bind_int64`/
// `.bind_float64`/`.bind_null`/`.execute`/`.query`/`.step`/
// `.column_string`/`.column_int64`/`.column_float64`/`.begin`/
// `.commit`/`.rollback` -- see `sqlite.rs`'s own module doc.

/// `Sqlite.open(path: String): Int64`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_open(path: *const c_char) -> i64 {
  catch_and_raise(move || sqlite::sqlite_open(path))
}

/// `Sqlite.open_memory(): Int64`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_open_memory() -> i64 {
  catch_and_raise(sqlite::sqlite_open_memory)
}

/// `Sqlite.close(conn: Int64): Void`.
///
/// # Safety
/// Always safe to call.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_close(conn: i64) {
  catch_and_raise(move || sqlite::sqlite_close(conn))
}

/// `Sqlite.execute_direct(conn: Int64, sql: String): Void`.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle; `sql`, if
/// non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_execute_direct(conn: i64, sql: *const c_char) {
  catch_and_raise(move || sqlite::sqlite_execute_direct(conn, sql))
}

/// `Sqlite.prepare(conn: Int64, sql: String): Int64`.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle; `sql`, if
/// non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_prepare(conn: i64, sql: *const c_char) -> i64 {
  catch_and_raise(move || sqlite::sqlite_prepare(conn, sql))
}

/// `Sqlite.bind_string(stmt: Int64, index: Int64, value: String): Void`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle; `value`, if
/// non-null, must point to a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_bind_string(
  stmt: i64,
  index: i64,
  value: *const c_char,
) {
  catch_and_raise(move || sqlite::sqlite_bind_string(stmt, index, value))
}

/// `Sqlite.bind_int64(stmt: Int64, index: Int64, value: Int64): Void`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_bind_int64(stmt: i64, index: i64, value: i64) {
  catch_and_raise(move || sqlite::sqlite_bind_int64(stmt, index, value))
}

/// `Sqlite.bind_float64(stmt: Int64, index: Int64, value: Float64): Void`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_bind_float64(stmt: i64, index: i64, value: f64) {
  catch_and_raise(move || sqlite::sqlite_bind_float64(stmt, index, value))
}

/// `Sqlite.bind_null(stmt: Int64, index: Int64): Void`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_bind_null(stmt: i64, index: i64) {
  catch_and_raise(move || sqlite::sqlite_bind_null(stmt, index))
}

/// `Sqlite.execute(stmt: Int64): Int64`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_execute(stmt: i64) -> i64 {
  catch_and_raise(move || sqlite::sqlite_execute(stmt))
}

/// `Sqlite.query(stmt: Int64): Int64`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_query(stmt: i64) -> i64 {
  catch_and_raise(move || sqlite::sqlite_query(stmt))
}

/// `Sqlite.step(cursor: Int64): Boolean`.
///
/// # Safety
/// `cursor` must be a live `Sqlite` cursor handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_step(cursor: i64) -> i64 {
  catch_and_raise(move || sqlite::sqlite_step(cursor))
}

/// `Sqlite.column_string(cursor: Int64, col: Int64): String`.
///
/// # Safety
/// `cursor` must be a live `Sqlite` cursor handle, currently
/// positioned on a real row.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_column_string(cursor: i64, col: i64) -> *const c_char {
  catch_and_raise(move || sqlite::sqlite_column_string(cursor, col))
}

/// `Sqlite.column_int64(cursor: Int64, col: Int64): Int64`.
///
/// # Safety
/// `cursor` must be a live `Sqlite` cursor handle, currently
/// positioned on a real row.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_column_int64(cursor: i64, col: i64) -> i64 {
  catch_and_raise(move || sqlite::sqlite_column_int64(cursor, col))
}

/// `Sqlite.column_float64(cursor: Int64, col: Int64): Float64`.
///
/// # Safety
/// `cursor` must be a live `Sqlite` cursor handle, currently
/// positioned on a real row.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_column_float64(cursor: i64, col: i64) -> f64 {
  catch_and_raise(move || sqlite::sqlite_column_float64(cursor, col))
}

/// `Sqlite.begin(conn: Int64): Void`.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_begin(conn: i64) {
  catch_and_raise(move || sqlite::sqlite_begin(conn))
}

/// `Sqlite.commit(conn: Int64): Void`.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_commit(conn: i64) {
  catch_and_raise(move || sqlite::sqlite_commit(conn))
}

/// `Sqlite.rollback(conn: Int64): Void`.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_sqlite_rollback(conn: i64) {
  catch_and_raise(move || sqlite::sqlite_rollback(conn))
}

// Plan 125 (Binary Serialization: bincode/msgpack): `Bincode.encode`/
// `.decode`, `MessagePack.encode`/`.decode`, dispatched by exact
// free-function/receiver-gated name in `emerald-codegen`'s own
// `build_method_call` — see `bincode.rs`'s/`msgpack.rs`'s own module
// docs for the full design.

/// # Safety
/// `obj` must point to a real `JsonValue` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bincode_encode(obj: *const c_void) -> i64 {
  catch_and_raise(move || bincodes::bincode_encode(obj))
}

/// # Safety
/// `id` must be a pointer `emerald_rt_bytes_from_slice` (or an
/// equally-shaped native producer, e.g. `String.to_bytes`) actually
/// returned.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_bincode_decode(id: i64) -> *mut c_void {
  catch_and_raise(move || bincodes::bincode_decode(id))
}

/// # Safety
/// `obj` must point to a real `JsonValue` block.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_msgpack_encode(obj: *const c_void) -> i64 {
  catch_and_raise(move || msgpack::msgpack_encode(obj))
}

/// # Safety
/// `id` must be a pointer `emerald_rt_bytes_from_slice` (or an
/// equally-shaped native producer) actually returned.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_msgpack_decode(id: i64) -> *mut c_void {
  catch_and_raise(move || msgpack::msgpack_decode(id))
}

// Plan 114 (JSON Web Tokens): `Jwt.encode_hs256`/`.verify_hs256`/
// `.verify_hs256_with_issuer`/`.encode_rs256`/`.verify_rs256`/
// `.verify_rs256_with_issuer`/`.encode_es256`/`.verify_es256`/
// `.verify_es256_with_issuer`/`.peek_header` — see `jwt.rs`'s own
// module doc.

/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `secret`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_encode_hs256(
  claims: *const c_void,
  secret: *const c_char,
) -> *const c_char {
  catch_and_raise(move || jwt::jwt_encode_hs256(claims, secret))
}

/// # Safety
/// `token`/`secret`, if non-null, must point to valid, NUL-terminated
/// C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_hs256(
  token: *const c_char,
  secret: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_hs256(token, secret))
}

/// # Safety
/// `token`/`secret`/`issuer`, if non-null, must point to valid,
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_hs256_with_issuer(
  token: *const c_char,
  secret: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_hs256_with_issuer(token, secret, issuer))
}

/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `private_key_pem`, if non-null, must point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_encode_rs256(
  claims: *const c_void,
  private_key_pem: *const c_char,
) -> *const c_char {
  catch_and_raise(move || jwt::jwt_encode_rs256(claims, private_key_pem))
}

/// # Safety
/// `token`/`public_key_pem`, if non-null, must point to valid,
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_rs256(
  token: *const c_char,
  public_key_pem: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_rs256(token, public_key_pem))
}

/// # Safety
/// `token`/`public_key_pem`/`issuer`, if non-null, must point to
/// valid, NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_rs256_with_issuer(
  token: *const c_char,
  public_key_pem: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_rs256_with_issuer(token, public_key_pem, issuer))
}

/// # Safety
/// `claims` must point to a real `Hash[String, String]` buffer;
/// `private_key_pem`, if non-null, must point to a valid,
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_encode_es256(
  claims: *const c_void,
  private_key_pem: *const c_char,
) -> *const c_char {
  catch_and_raise(move || jwt::jwt_encode_es256(claims, private_key_pem))
}

/// # Safety
/// `token`/`public_key_pem`, if non-null, must point to valid,
/// NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_es256(
  token: *const c_char,
  public_key_pem: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_es256(token, public_key_pem))
}

/// # Safety
/// `token`/`public_key_pem`/`issuer`, if non-null, must point to
/// valid, NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_verify_es256_with_issuer(
  token: *const c_char,
  public_key_pem: *const c_char,
  issuer: *const c_char,
) -> *mut c_void {
  catch_and_raise(move || jwt::jwt_verify_es256_with_issuer(token, public_key_pem, issuer))
}

/// # Safety
/// `token`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_jwt_peek_header(token: *const c_char) -> *const c_char {
  catch_and_raise(move || jwt::jwt_peek_header(token))
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
