2026-09-21T20:01:00Z

---
name: FFI/ABI Conventions for Rust-Backed Native Functions
overview: "The reusable calling convention every `emerald-rt` export from plan 96 onward must follow, generalizing plan 91's single hand-written proof function into a documented, partly macro-enforced contract: `emerald_rt_<module>_<fn>` naming, a `catch_unwind`-wrapped body on every export, a zero-conversion `*const c_char` convention for plain-text `String` matching plan 59's existing FFI precedent exactly, an explicit `(ptr: *const u8, len: i64)` ABI-level convention for binary-safe data with its real, disclosed gaps named rather than hidden, and a canonical rule for when a native failure becomes an Emerald `Result[T, E]` (plan 53) versus a raised exception (plan 11) — decided here once so no later domain plan re-litigates it."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-rename-to-naming-convention
    content: "Rename plan 91's one shipped export, `emerald_rt_fnv1a_hash`, to `emerald_rt_string_fnv1a_hash` in `crates/emerald-rt/src/lib.rs`, and update every call site that names it literally: the `module.add_function(\"emerald_rt_fnv1a_hash\", ...)` declaration in `emerald-codegen` (plan 91's `leaf-panic-boundary-and-fnv1a-proof`), the `#[test]` in `emerald-rt` asserting its FNV-1a value, and `examples/rust_native_runtime_proof.em`'s expected-output table entry if the example harness names the symbol anywhere. Add a doc comment at the top of `crates/emerald-rt/src/lib.rs` stating the mandatory `emerald_rt_<module>_<fn>` naming rule in prose, citing this plan by number, so it reads as an enforced convention rather than an accident of plan 91's single function happening to be named that way. `<module>` is the lowercase noun the function's Emerald-facing surface groups under (`string`, `file`, `http`, `sqlite`, ...) — not the Rust crate name (`emerald_rt` is already the fixed prefix) and not the wrapped third-party crate's own name (which may change across a re-vetting per plan 95 without the ABI-visible symbol needing to change)."
    status: done
  - id: leaf-panic-boundary-macro
    content: "Add a `macro_rules! emerald_rt_fn` (or equivalently a documented helper function taking a closure) to `crates/emerald-rt/src/lib.rs` that wraps `std::panic::catch_unwind` around a function body and, on a caught panic, converts the panic payload (downcast `&str`/`String`, falling back to a fixed `\"native panic (no message)\"` literal for a non-string payload per `std::panic::catch_unwind`'s own documented payload contract) into a call to `leaf-native-error-and-panic-raise`'s `emerald_rt_raise_native_error` rather than returning a sentinel value. Rewrite `emerald_rt_string_fnv1a_hash` (renamed above) to go through this macro, replacing plan 91's placeholder `.unwrap_or(-1)` — its function body never actually panics today (FNV-1a is total over any byte slice), so this rewrite is a pure convention change with no behavior difference for that one function, proven by its existing `#[test]` continuing to pass unmodified."
    status: done
  - id: leaf-native-error-and-panic-raise
    content: "Add a compiler-synthesized `NativeError` class (one field, `message: String`, mirroring the `Option[T]` enum's own 'a compiler-synthesized... name in `classes`' precedent noted in `grammar.lalrpop`'s own comment above the `own`/`borrow` `TypeExpr` productions) registered in `emerald-sema`'s `classes` map and `emerald-codegen`'s class-tag-assignment pass at a fixed, reserved class tag (tag `0`) assigned *before* any user-declared class from `program.items` gets a tag, so `NativeError`'s numeric identity is identical in every compiled Emerald program and safe for `emerald-rt` to hardcode as a Rust `const NATIVE_ERROR_TAG: i64 = 0;` with no access to that specific compilation's class registry. Add `emerald_rt_raise_native_error(msg: *const c_char) -> !` to `crates/emerald-rt/src/lib.rs`, implemented by declaring `emerald_alloc`/`emerald_raise` as `unsafe extern \"C\"` inside `emerald-rt` itself (both symbols are already exported from the linked-in C archive per plan 91's dual-archive build — no new marshaling boundary, the same runtime entry point codegen's own `Stmt::Raise` lowering already calls, per plan 11) and calling `emerald_raise(NATIVE_ERROR_TAG, alloc_and_store_message(msg))`. A `.em` regression test rescues a deliberately panicking native call (see the plan's own concrete proof for a candidate) with an ordinary `rescue NativeError => e; puts e.message`, proving the boundary is a real, catchable Emerald exception and not merely documented prose."
    status: done
  - id: leaf-result-construction-helpers
    content: "Add `emerald_rt_result_ok(payload: i64) -> *mut c_void` and `emerald_rt_result_err(msg: *const c_char) -> *mut c_void` to `crates/emerald-rt/src/lib.rs`, each allocating exactly plan 53's own `Result[T, E]` layout (16 bytes via the same cross-archive `emerald_alloc` call as the leaf above: `[discriminant: i64 @ offset 0][payload: i64 @ offset 8]`, `0`/`1` respectively) so the pointer either helper returns is byte-for-byte what `emerald-codegen`'s own `Expr::Ok`/`Expr::Err` arms already produce — a native function declaring `Result[Int64, String]` as its LLVM return type needs zero additional marshaling at the call site, the same free convergence plan 59 found between `CString`/`String?` and plan 43's nullable-pointer representation. Add `String.fnv1a_hash_checked(self): Result[Int64, String]` to `emerald-rt` (extending plan 91's own proof function, not a new domain) returning `Err(\"input must not be empty\")` for `\"\"` and `Ok(<same FNV-1a value as .fnv1a_hash>)` otherwise, dispatched through the identical `ValKind::Str`-gated intrinsic mechanism plan 91's `leaf-panic-boundary-and-fnv1a-proof` already established for `.fnv1a_hash`."
    status: done
  - id: leaf-binary-safe-buffer-convention
    content: "Document, in a new `crates/emerald-rt/src/lib.rs` module-level doc section (no new Emerald-facing type introduced by this plan — see Decision log), the `(ptr: *const u8, len: i64)` two-parameter ABI shape every future domain plan needing embedded-NUL or non-UTF-8 byte data (compression, crypto, binary serialization, images) must use in place of a bare `*const c_char`, on both the parameter and return side (a returning function takes an additional `out_len: *mut i64` out-parameter, since a bare two-word `#[repr(C)] struct` return is not guaranteed to pass in registers identically across every future host ABI this project may target, whereas an out-parameter is the same convention `emerald_string_length`-adjacent runtime helpers already use for multi-value returns). Document the exact, disclosed limitation this plan does not solve: an Emerald `String` value crossing this convention as an input is still bounded by `emerald_string_length`'s own `strlen`-based length (plan 59's own finding — `String` has no length header), so a domain plan built on this convention can *consume* arbitrary bytes a native call *produces*, but cannot yet accept an Emerald-source-literal `String` containing an embedded NUL byte — that requires a real Emerald-level `Bytes`/binary-literal type, named explicitly in 'Not yet decided' below rather than assumed solved."
    status: done
  - id: leaf-example-and-gate
    content: "Extend `examples/rust_native_runtime_proof.em` (or add `examples/ffi_abi_conventions_proof.em`, wired into `emerald-cli/tests/examples.rs`'s CI-checked table per plan 91's own precedent) with this plan's concrete proof below, exercising the renamed `.fnv1a_hash`, the new `.fnv1a_hash_checked` on both an empty and a non-empty input, and a deliberately panicking native call caught via `rescue NativeError => e`. Run the full `AGENTS.md` gate: `cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`, plus a clean-checkout `cargo build` end-to-end, matching plan 91's own `leaf-example-and-full-gate` bar exactly."
    status: done
isProject: false
---

# Plan 92 — FFI/ABI Conventions for Rust-Backed Native Functions

Plan 91 built one proof function, `emerald_rt_fnv1a_hash`, and proved
exactly one convention in working code: a panic must never unwind past
an `extern "C"` boundary into the C runtime's `setjmp`/`longjmp` frames,
so every export's body is wrapped in `std::panic::catch_unwind`. Every
other convention a real domain plan needs — what an export is *named*,
how a `String` crosses the boundary, how binary data that isn't valid
UTF-8 or contains an embedded NUL crosses it, and what a *caught*
failure becomes on the Emerald side — was explicitly deferred to this
plan, both in plan 91's own Decision log ("full design in plan 92") and
its Out of scope bullet ("no error-code/`Result` marshaling convention
beyond this one function's `-1`-on-panic placeholder — that is plan 92's
job, generalized"). Plans 96-191 each add a handful of new
`emerald_rt_*` exports; if each domain plan invents its own naming
scheme, its own answer to "does this failure raise or return `Err`," and
its own ad hoc binary-data convention, the 96-domain-plan archive
becomes an unreadable pile of one-off decisions instead of one crate
with one shape. This plan is the single place that shape gets decided,
cited by number from every domain plan the way plan 91 itself is.

This plan does not touch `emerald_runtime.c` or any C code — it is
entirely inside `crates/emerald-rt/` (extending, not replacing, plan
91's own crate) and the two small `emerald-sema`/`emerald-codegen` hooks
`leaf-native-error-and-panic-raise` needs to register one new
compiler-synthesized class. It adds no third-party Rust dependency —
the same "prove the convention on code whose own correctness can never
be in question" posture plan 91 adopted for FNV-1a applies here to
FNV-1a's checked sibling and to a deliberately-panicking function; the
crate-vetting discipline for actual third-party crates is plan 95's job,
not this one's.

## Concrete proof this plan targets

```ruby
puts "hello".fnv1a_hash

empty_result: Result[Int64, String] = "".fnv1a_hash_checked
case empty_result
when Ok(v)
  puts v
when Err(e)
  puts e
end

ok_result: Result[Int64, String] = "hello".fnv1a_hash_checked
case ok_result
when Ok(v)
  puts v
when Err(e)
  puts e
end

begin
  "trigger".fnv1a_hash_panic_for_test
rescue NativeError => e
  puts e.message
end
```

Expected output, in order: the real FNV-1a-32 hash of `"hello"` (the
same numeric value plan 91's own proof prints, confirming the rename in
`leaf-rename-to-naming-convention` changed no behavior); the literal
string `input must not be empty` (the `Err` path of
`.fnv1a_hash_checked("")`); the same FNV-1a hash of `"hello"` a second
time (the `Ok` path, proving the checked and unchecked variants agree);
and a fixed, disclosed diagnostic string (e.g. `native panic in
fnv1a_hash_panic_for_test`) from `e.message` — the one line in this
proof that exercises `leaf-native-error-and-panic-raise` end to end, not
just construction. `.fnv1a_hash_panic_for_test` is a `#[cfg(test)]`-free
but deliberately-trivial *real* exported function added only by this
plan's own leaf, whose entire body is `panic!("native panic in {}",
"fnv1a_hash_panic_for_test")` behind the `emerald_rt_fn!` macro — it
exists purely so this plan's own proof can demonstrate a genuine
Rust-side panic converting into a genuine, rescuable Emerald exception,
the same "prove it in code, not just prose" posture plan 91 used for
`catch_unwind` itself.

## Decision log

- **Naming: `emerald_rt_<module>_<fn>`, retroactively applied to plan
  91's own function.** Plan 91 shipped exactly one export,
  `emerald_rt_fnv1a_hash`, with no `<module>` segment — defensible for a
  crate with one function, indefensible once dozens of domain plans add
  their own. `leaf-rename-to-naming-convention` renames it to
  `emerald_rt_string_fnv1a_hash` specifically so this plan's own
  worked proof and every citing domain plan from 96 onward see one
  consistent pattern in the one crate that exists today, rather than "do
  what plan 92 says, except the one function plan 91 already shipped,
  which is grandfathered." `<module>` groups by the function's
  Emerald-facing surface (`string`, `file`, `http`), never by the
  wrapped crate's own name — plan 95's crate-vetting ledger tracks which
  third-party crate backs a given module, and a module's backing crate
  can be swapped by a future re-vetting without renaming every ABI
  symbol a compiled Emerald program's object file already references.
- **The `catch_unwind` boundary moves from "a convention documented in
  plan 91's prose, proven once by hand" to "a shared macro every future
  export is written through."** Plan 91 itself is explicit that
  `panic = "unwind"` (not `"abort"`) was chosen specifically so a panic
  three dependencies deep in some future crate doesn't kill the whole
  process — but that protection only holds if every one of the ~96
  domain plans' exports actually remembers to wrap its own body. A
  documented convention a human has to remember at each of ~200+ future
  call sites is a real, foreseeable failure mode; `emerald_rt_fn!`
  (`leaf-panic-boundary-macro`) makes forgetting it a compile error
  instead of a silent gap — writing `#[no_mangle] pub extern "C" fn ...`
  directly, bypassing the macro, is still physically possible (Rust has
  no way to forbid it), but plan 95's acceptance checklist (item 3,
  "FFI/ABI notes citing plan 92") is the enforcement point: a domain
  plan's own leaf descriptions must show the macro in use, and a review
  pass checks for it the same way it checks any other convention.
- **A caught panic becomes a real, rescuable `NativeError` exception —
  never plan 91's placeholder sentinel, and never a silent abort.**
  This directly closes plan 91's own named gap ("this one function's
  `-1`-on-panic placeholder"). `NativeError` is a single-field
  (`message: String`) compiler-synthesized class, the same kind of
  compiler-introduced-but-source-invisible declaration `Option[T]`
  already is (verified this session: `grammar.lalrpop`'s comment above
  the `own`/`borrow` `TypeExpr` productions calls `Option[T]` "an
  ordinary `Ident` matching a compiler-synthesized enum name in
  `classes`" — precedent for a name that resolves against the real
  class/enum registry without a matching source-level declaration
  anywhere). Its class tag must be **fixed and reserved**, not assigned
  by `program.items`' declaration order the way every user class's tag
  is today (plan 11's Decision log, verified: "each class gets a stable
  integer class tag... instead of string/RTTI... assignment order in
  `program.items`") — `emerald-rt` has no access to any particular
  compilation's `program.items` order, so it cannot know at Rust-compile
  time what tag a *user's* class registration pass would have assigned
  `NativeError` if it were an ordinary class. Reserving tag `0` for it,
  assigned before the first user class in codegen's registration pass,
  makes the tag a real Rust `const` both sides agree on by construction,
  not a value that has to be threaded through the FFI call itself.
- **Panics reuse the exact runtime entry points codegen's own `raise`
  lowering already calls — no new Rust-side allocator or exception
  mechanism.** Plan 91's own Decision log observes that its two
  archives are linked into one final binary and that "neither
  references the other's symbols *yet*" — this plan is the first to
  actually cross that seam deliberately: `crates/emerald-rt/src/lib.rs`
  declares `unsafe extern "C" { fn emerald_alloc(size: i64) -> *mut
  c_void; fn emerald_raise(tag: i64, ptr: *mut c_void) -> !; }` and
  calls them directly, the same two symbols `Stmt::Raise`'s own codegen
  lowering already emits calls to (plan 11, verified: `emerald_raise`
  is called with a class tag and an allocated instance pointer). This
  is strictly less machinery than giving `emerald-rt` its own
  allocation strategy or its own `setjmp`/`longjmp`-compatible unwind
  mechanism — it reuses the one the C runtime already has, exactly the
  same "generalize what already exists" posture plan 91 used for the
  dual-archive link step itself and plan 59 used for extern-function
  declarations.
- **`Result[T, E]` construction from Rust reuses plan 53's exact 16-byte
  layout, for the identical reason.** `emerald_rt_result_ok`/
  `emerald_rt_result_err` allocate `[discriminant: i64 @0][payload: i64
  @8]` via the same cross-archive `emerald_alloc` call, discriminant `0`
  for `Ok`, `1` for `Err` — verified against plan 53's own Decision log
  ("a fixed, hard-coded compound `Type` variant... `[discriminant: i64
  @ offset 0][payload: 8 bytes @ offset 8]`, `discriminant = 0` for
  `Ok`, `1` for `Err`"). A native function declaring `Result[Int64,
  String]` as its return type needs no codegen-side unwrapping or
  reboxing at its call site — the pointer it returns is already a
  legal `Result[T, E]` value by construction, consumable directly by an
  ordinary `case ... when Ok(v) ... when Err(e) ... end`, the same
  "zero marshaling because the representations already agree" outcome
  plan 59 found for `CString`/`String?` against plan 43's nullable
  pointers.
- **The Result-vs-exception choice is a rule, not a per-call judgment
  call: an anticipated failure the wrapped crate's own API already
  distinguishes (a parse error, a connection refused, a file not found)
  is always `Result[T, E]`; a genuine Rust panic caught at the
  `catch_unwind` boundary is always a raised `NativeError` exception,
  never a `Result`.** This is a direct, deliberate application of plan
  53's own two-channel model, verified against its Decision log:
  "`Result[T, E]` is for expected, recoverable failures a caller is
  meant to handle... [exceptions are for] exceptional, programmer-error
  conditions... a caller usually isn't expected to routinely check for
  at every call site." A Rust panic is by construction *not* something
  the wrapped crate's own author anticipated as a normal outcome of
  calling that function correctly (a well-behaved crate returns
  `Result`/`Option` for its own expected failure modes and panics only
  on a genuine invariant violation or bug) — so it belongs on the same
  channel plan 53 reserves for programmer-error conditions, never mixed
  into the `Result[T, E]` a domain plan's own signature declares for its
  expected failure modes. A domain plan's job, per this rule, is to map
  the wrapped crate's own `Result`/`Option` outcomes onto
  `emerald_rt_result_ok`/`emerald_rt_result_err`, and let anything that
  actually panics fall through to `catch_unwind` unmodified.
- **`String` needs zero conversion crossing this boundary, in either
  direction — reusing plan 59's finding verbatim rather than
  re-deriving it.** Plan 59 verified, directly against
  `runtime/emerald_runtime.c` and `emerald-codegen`'s own `ValKind` doc
  comment, that an Emerald `String` is "a plain, `malloc`-backed,
  null-terminated buffer" — bit-for-bit identical to C's own `char*`
  convention. Rust's own `*const std::os::raw::c_char` has the
  identical wire representation (a plain pointer; the type exists
  purely for Rust's own type-checking, not a different runtime shape),
  so an `emerald_rt_*` function taking or returning plain text uses
  `*const c_char`/`CStr`/`CString` exactly as any other C FFI boundary
  would, with the same trust caveat plan 59 already disclosed for
  return-side pointers (a returned `*mut c_char` must itself be a real,
  `emerald_alloc`-backed, null-terminated, valid-UTF-8 buffer — never a
  Rust `String`'s own heap allocation, which is not freed the way
  `emerald_alloc`'d memory is treated project-wide, since this project
  has no `free` anywhere per plan 51's own finding).
- **The `(ptr: *const u8, len: i64)` binary-safe convention is fixed at
  the ABI level only, in this plan — it does not yet give Emerald
  source a way to construct a literal containing an embedded NUL
  byte, and this plan states that gap rather than papering over it.**
  A `String` value's own length, wherever this plan's helpers need one,
  is still whatever `emerald_string_length`'s `strlen`-based
  implementation computes (plan 59's own finding, `runtime/
  emerald_runtime.c`'s `emerald_string_length`) — a domain plan can
  *consume* raw bytes a Rust function *produces* (compressed output, a
  hash digest, a decrypted buffer) through this convention with no
  ceiling on content, but cannot yet *accept* Emerald-source-literal
  binary data past the first embedded NUL, because no Emerald-level
  type with real length metadata exists for it to originate from (plan
  45's own finding: `Array[T]` also carries no runtime length metadata
  today). Closing this — a real `Bytes`/binary-literal Emerald type — is
  explicitly out of scope here; see 'Not yet decided' below. The
  out-parameter shape (`out_len: *mut i64` on the return side, rather
  than a `#[repr(C)]` two-word struct return) is chosen over a struct
  return because this project's dual-archive link step (plan 91) has
  not yet had to reason about any host ABI's small-struct-return-in-
  registers rules across every target this project claims to support,
  and an out-parameter sidesteps that question entirely by never
  returning an aggregate — the same reasoning `emerald_bytes_
  outstanding()`-style single-scalar-return helpers already use
  throughout `emerald_runtime.c`.
- **Out of scope.** No new Rust third-party dependency (plan 95's job,
  applied per domain from plan 96 onward); no Emerald-facing `Bytes`/
  binary-buffer language type (named explicitly above and in 'Not yet
  decided'); no change to `String`'s own core representation (plan
  19/45's null-terminated, no-length-header design stands, unmodified,
  for the third time this batch); no resource-handle/lifetime story for
  native objects that outlive a single call (plan 93's job entirely —
  every function this plan's own proof adds is a pure, stateless
  computation with nothing to hold open between calls); no async/
  blocking-runtime bridging (plan 94's job — FNV-1a and its checked
  sibling are synchronous CPU work, same as plan 91's own proof).

## Not yet decided (blocking EXECUTE)

1. Whether `NativeError` needs any field beyond a bare `message:
   String` — an error "kind"/code enum distinguishing, say, "the
   wrapped crate panicked" from "an internal invariant this plan's own
   macro violated" is a real, plausible future need, but inventing that
   taxonomy now, with a total of two exported functions in the whole
   crate to motivate it, would be speculative. Left for whichever
   domain plan first needs to `rescue` different native-panic
   conditions differently.
2. Whether a genuine Emerald-facing `Bytes`/binary-literal type (fixing
   the embedded-NUL construction gap named above) belongs to this
   batch at all, or is deferred indefinitely until a specific domain
   plan (compression, crypto, or an image codec are the likely first
   candidates per the batch's own framing) actually needs to construct,
   not just consume, non-UTF-8 Emerald-source data. This plan
   deliberately does not decide that question — it only ensures the
   ABI-level convention such a type would eventually marshal through
   already exists and is documented.
3. The exact out-parameter register/calling convention for a
   `(ptr, len)`-returning function on every host target this project
   claims to support — deferred the same way plan 91 deferred its own
   platform-library flag list, "confirm empirically" at the point a
   real domain plan first returns binary data, rather than asserted
   here with no function yet exercising it.

## Update (2026-09-22, same-day session): implemented, all six leaves done

Full workspace gate green: `cargo nextest run --workspace` (932/932,
2 pre-existing skips), `cargo clippy --workspace --all-targets` (clean),
`treefmt` (0 changed), a clean-checkout `cargo build -p emerald-cli`
end to end. `examples/rust_native_runtime_proof.em` extended in place
(not a new file) and run through the real CLI, printing the full
expected six-line sequence.

Three real, disclosed deviations from this plan's own text, each found
only by actually building/running it:

- **`emerald_rt_fn!` became a plain function, `catch_and_raise`, not a
  `macro_rules!`.** The plan's own `leaf-panic-boundary-macro` asked
  for a macro specifically so forgetting the panic boundary is a
  compile error. A generic function (`fn catch_and_raise<T>(body: impl
  FnOnce() -> T + UnwindSafe) -> T`) gives the identical "every export
  calls one shared wrapper" shape with no macro-hygiene surface to
  maintain across ~200 future call sites; `emerald_rt_string_fnv1a_
  hash`/`_checked`/`_panic_for_test` all go through it identically to
  how they'd go through a macro. Revisit if a future domain plan finds
  a real case the function form can't express that a macro could.
- **`NativeError` needed a synthesized `message` GETTER METHOD, not
  just a registered field.** The plan's own Decision log registers
  `NativeError` as a `ClassInfo`/`ClassDef` with a `message: String`
  field and left it there. Tried first: `rescue NativeError => e; puts
  e.message` — sema rejected it outright (`class NativeError has no
  method message`), because this language's own field-access
  convention (confirmed against `examples/exceptions.em`'s `MyError`,
  which hand-writes `fn code: Int64 do @code end`) requires an
  explicit getter METHOD for every external field read; there is no
  bare-field-access expression form at all. Fixed by registering a
  synthesized `message` method in both `emerald-sema`'s `ClassInfo.
  methods` (a hand-built `FunctionSig`) and `emerald-codegen`'s
  `build_method_call` (a special case reading
  `ctx.classes["NativeError"].fields["message"]` directly via the
  existing `load_field`/`field_ptr` helpers, checked before the
  ordinary `method_owners`
  lookup — the same pattern `to_cstring`'s own type-level-relabeling
  special case and the actor `register` special case immediately above
  it already establish) — no `AstFunction`/LLVM function body is ever
  actually generated for it, since there's no source declaration to
  compile one from.
- **`catch_unwind`'s own `Err` payload did not reliably downcast to
  `&str`/`String` in the actual `--release`-built archive this project
  links in — despite an in-process `cargo test -p emerald-rt` unit test
  of the IDENTICAL `panic!("literal")` shape succeeding.** Found only
  by running the real `.em` example through the real CLI: `e.message`
  printed the fixed fallback string ("native panic (no message)")
  instead of the real panic text. A temporary debug probe (`payload.
  is::<&str>()`/`.is::<String>()`, both false) confirmed the payload's
  concrete type genuinely isn't either — root cause not fully
  identified (a real, open question, not resolved by this plan), but
  reproduced consistently across separate runs. Fixed by sidestepping
  the payload downcast entirely: `install_panic_hook_once` installs a
  real `std::panic::set_hook` (chaining to, not replacing, the previous
  hook) that captures `PanicHookInfo`'s own rendered message into a
  thread-local (`LAST_PANIC_MESSAGE`) `panic_message` reads first,
  falling back to the original downcast only if the hook somehow never
  ran. `PanicHookInfo` has no stable `.message()` accessor on this
  project's rustc (confirmed by a real, empirical `E0599`), so the
  message is recovered by splitting `info.to_string()`'s own `"panicked
  at {location}:\n{message}"` rendering on its first newline — verified
  correct against the real captured output, not assumed from
  documentation.

Also found and fixed along the way (not a plan-92 deviation, a plain
mechanical necessity for its own Concrete Proof): a `Result[T, E]`
`match` scrutinee must be a plain local per the grammar's own
`<scrutinee:CondExpr>` production combined with codegen's own internal-
error check ("`Ok`/`Err` match scrutinee must be a plain local"), so
each `.fnv1a_hash_checked` call is bound to a `Let` before the `match`,
not inlined — and the actual match syntax is `match <scrutinee> do
Ok(<var>) do ... end Err(<var>) do ... end end`, not the plan text's
own `case`/`when` sketch (`examples/rust_native_runtime_proof.em`'s own
header comment discloses both).
