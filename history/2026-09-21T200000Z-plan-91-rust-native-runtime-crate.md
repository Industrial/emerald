2026-09-21T20:00:00Z

---
name: Rust-Native Runtime Crate — A Second, Rust-Compiled Static Archive Alongside `emerald_runtime.c`
overview: "A new workspace crate, `crates/emerald-rt`, compiled with `crate-type = [\"staticlib\", \"rlib\"]` and linked into every user program's final binary alongside — not instead of — the existing `runtime/emerald_runtime.c` archive, so that all future stdlib expansion (plans 92-191: HTTP, TLS, crypto, databases, compression, and everything else this batch adds) can be written in real, dependency-checked Rust rather than hand-rolled C. This plan is purely the load-bearing mechanism — one hand-written, zero-dependency proof function (an FNV-1a string hash) — not any user-facing stdlib surface; plan 92 builds the calling convention this crate's functions must follow, and every domain plan from 96 onward is a tenant of the archive this plan makes buildable."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-emerald-rt-crate
    content: "Create `crates/emerald-rt/Cargo.toml` with `[lib] crate-type = [\"staticlib\", \"rlib\"]`, `[profile.release] panic = \"unwind\"` (NOT `\"abort\"` — see Decision log; this is the one workspace member that must keep unwind support so `std::panic::catch_unwind` at every exported function's boundary actually works), and add it to the workspace `[members]` list in the root `Cargo.toml` alongside the existing nine crates (emerald-lexer/parser/sema/codegen/driver/cli/lsp/mcp/fmt). Add `emerald-rt` as an ordinary `[dependencies]` path entry of `crates/emerald-driver/Cargo.toml` so it participates in the normal workspace build graph."
    status: pending
  - id: leaf-json-artifact-discovery-in-build-rs
    content: "Extend `crates/emerald-driver/build.rs` with a second compilation step alongside its existing `cc::Build` call: invoke `std::env::var(\"CARGO\").unwrap_or_else(|_| \"cargo\".into())` (never a hardcoded `\"cargo\"` literal — the running Cargo's own path is the one guaranteed-correct choice, exactly as Cargo's own build-script documentation states) as `Command::new(cargo).args([\"build\", \"--message-format=json\", \"--release\", \"-p\", \"emerald-rt\", \"--target-dir\", <a dedicated subdirectory of OUT_DIR>])`, parse the resulting newline-delimited JSON stream for the `compiler-artifact` message whose `target.name == \"emerald_rt\"` and whose `target.kind` contains `\"staticlib\"`, and take that message's `filenames` entry ending in `.a` as the real artifact path — this is the same artifact-discovery technique `corrosion` (CMake<->Cargo) and `cxx-build` use for embedding a Rust static library into a build driven by a different tool, applied here to a build script embedding one crate's artifact into a *sibling* crate's `build.rs`, which stable Cargo has no first-class API for otherwise (Cargo's `-Zbindeps` artifact-dependencies feature solves this natively but is nightly-only; this project's toolchain is pinned to a stable channel per `rust-devenv`, so the JSON-parsing subprocess route is the real, precedented, stable-compatible mechanism, not an invented workaround)."
    status: pending
  - id: leaf-embed-and-link-second-archive
    content: "Mirror the existing C-archive pattern exactly: emit `cargo:rustc-env=EMERALD_RT_ARCHIVE=<discovered .a path>` from `build.rs`, `include_bytes!` it in `emerald-driver/src/lib.rs` next to the existing `EMERALD_RUNTIME_ARCHIVE` embed, write both archives out to temp files at link time, and pass both to the same final `cc` invocation (`crates/emerald-driver/src/lib.rs`'s `link` function, `-no-pie <obj> <c_runtime.a> <rust_rt.a> -o <output>` — Rust-archive-after-C-archive is not required by link order here since neither references the other's symbols yet, but keep C-archive-before-Rust-archive for now to match today's argument order and minimize the diff). Add whatever platform libc-adjacent flags a `staticlib`-crate-type Rust archive needs at plain-`cc` link time on this project's supported targets (Linux: `-lpthread -ldl -lm`; confirm empirically via the proof example below rather than asserting a full list untested — glibc's dynamic linker will name any missing symbol at link time if the list is incomplete, which is itself the verification method)."
    status: pending
  - id: leaf-panic-boundary-and-fnv1a-proof
    content: "Write `crates/emerald-rt/src/lib.rs` with one real exported function: `#[no_mangle] pub extern \"C\" fn emerald_rt_fnv1a_hash(s: *const std::os::raw::c_char) -> i64`, whose *entire* body is `std::panic::catch_unwind(|| { /* real FNV-1a over the CStr's bytes */ }).unwrap_or(-1)` — establishing, in working code, the catch-unwind-at-every-boundary convention plan 92 will mandate project-wide, not just asserting it in prose. Declare it in `crates/emerald-codegen` the same way plan 59 found the ~30 existing C runtime functions already declared (`module.add_function(\"emerald_rt_fnv1a_hash\", i64_ty.fn_type(&[ptr_ty.into()], false), Some(Linkage::External))`), and expose it to Emerald source as `String.fnv1a_hash(self): Int64`, dispatched via the same receiver-storage-kind intrinsic mechanism plan 45 already established for `.upcase`/`.length`. Add a `#[test]` in `emerald-rt` itself (run via `cargo nextest run --workspace`, proving the `rlib` crate-type half of `leaf-scaffold-emerald-rt-crate` actually works, not just the `staticlib` half) asserting the same function's Rust-side return value against a hand-computed FNV-1a value for a fixed known string."
    status: pending
  - id: leaf-example-and-full-gate
    content: "Add `examples/rust_native_runtime_proof.em` (the Concrete Proof below) to `examples/` and wire it into `emerald-cli/tests/examples.rs`'s CI-checked table (`examples/README.md`'s own stated contract: 'every file there is compiled, run, and asserted against its real output on every push'). Run the full `AGENTS.md`-stated gate — `cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`, and confirm a totally clean checkout (`cargo clean` or a fresh `git clone` into a scratch dir) can still build `emerald-cli` end-to-end with no manual step beyond `cargo build` — this last check is the one that actually proves the JSON-artifact-discovery build.rs mechanism is real and not merely 'works on a warm target dir'."
    status: pending
isProject: false
---

# Plan 91 — Rust-Native Runtime Crate

Every plan from 96 onward in this batch (91-191) exists to answer one
2026-09-21 session directive plainly: Emerald's native runtime today is
one file, `runtime/emerald_runtime.c` (plan 06's original codegen target,
extended by nearly every plan since — plan 45's string/IO helpers, plan
51's arena allocator, plan 54/55/57's actor model, plan 60's distributed
networking), and every one of its roughly thirty exported functions is
hand-written C, verified directly against the file this session. The
directive is explicit: prefer exposing vetted, widely-used Rust crates as
native Emerald functions over writing more C, or over reimplementing
what a mature crate already does correctly. That is a real architectural
fork, not an incremental addition — it requires a second compiled
artifact, built by `cargo`/`rustc` rather than `cc`, embedded and linked
exactly as portably as the first. This plan builds that mechanism, and
only that mechanism: one crate, one exported function, zero third-party
dependencies, chosen specifically so its own correctness can never be in
question — the point is proving the *pipe*, not yet anything flowing
through it.

## Concrete proof this plan targets

```ruby
puts "hello".fnv1a_hash
puts "hello world".fnv1a_hash
puts "".fnv1a_hash
```

Expected output: three `Int64` values — the real FNV-1a-32 (widened to
`Int64`, matching Emerald's real type system's lack of any narrower
integer, per plan 59's own finding) hash of `"hello"`, `"hello world"`,
and the empty string, computed identically by Emerald's own compiled
output and by the `emerald-rt` crate's own `#[test]`. The three values
themselves are not the point; that a plain `cargo build` from a clean
checkout produces a working `emerald-cli` that runs this program
correctly, with no manual pre-build step, is.

## Decision log

- **The existing C runtime's own bundling design is the direct model
  this plan generalizes, not a break from it.** Verified this session,
  read in full: `crates/emerald-driver/build.rs`'s own doc comment
  states its purpose precisely — compile `runtime/emerald_runtime.c` "at
  this crate's own build time... so the *shipped binary*... no longer
  needs this repo's `runtime/emerald_runtime.c` present at
  link-a-user's-program time," by `include_bytes!`-embedding the
  resulting archive's real bytes rather than merely recording its
  `OUT_DIR` path (which "doesn't exist once the binary is copied
  elsewhere"). Every design constraint that build.rs already satisfies
  for the C archive — no repo access needed at a user's link time, a
  redistributable `emerald-cli` binary — this plan's Rust archive must
  satisfy identically. The only genuinely new problem is *how* to
  produce that second archive, since `cc::Build::new().compile(...)`
  has no Rust equivalent.
- **`-ffunction-sections`/`-fdata-sections` + `--gc-sections` is a
  C-toolchain-specific fix for a problem Rust's own compiler already
  solves differently, so it is not carried over unmodified.** The C
  build.rs's own comment explains why those flags exist: "`emerald_
  runtime.c` is one translation unit compiled to one `.o`... the linker
  can only pull in a static-archive member whole-or-nothing" without
  them. Rust's compilation model has no equivalent one-translation-unit
  problem — `rustc` already emits one object-file-section (and,
  post-LTO within the crate, one optimized unit) per function by
  default in a release build, and per-symbol dead-code elimination at
  the final `cc` link step works on a `staticlib`'s `.a` the same way
  it does on any other static archive member. This plan does not add
  `-ffunction-sections`-equivalent flags to `emerald-rt`'s own build —
  there is nothing missing to add — but does keep the final `cc`
  invocation's `--gc-sections` (already implied by the existing link
  step, per the C build.rs's own comment attributing it to `build_link_
  args`/`link_many` in `emerald-driver/src/lib.rs`/`src/parallel.rs`)
  unchanged, since it benefits the new archive for free: a program using
  only `String.fnv1a_hash` and nothing else from the eventual 100+-plan
  stdlib surface must not pay for the parts of `emerald-rt` it never
  calls, exactly as today's C runtime already guarantees for its own
  functions.
- **Recursive `cargo build` from inside a build script is a real,
  named anti-pattern this plan adopts deliberately, not accidentally —
  the honest tradeoff is stated here, not hidden.** Cargo's own book
  cautions against invoking `cargo` from a `build.rs` (lock contention,
  environment-variable leakage between the outer and inner invocation,
  unclear caching behavior). This plan does it anyway, for the same
  reason `corrosion` and `cxx-build` do: stable Cargo has no other
  supported way, today, for one crate's build script to obtain another
  crate's compiled `staticlib` artifact path. Cargo's actual native
  solution to this exact problem — artifact dependencies (`[dependencies]
  emerald-rt = { path = "...", artifact = "staticlib" }`, no build.rs
  subprocess needed at all) — exists but is gated behind
  `-Zbindeps`, nightly-only as of this writing. This project's toolchain
  is pinned to a stable release (per the `rust-devenv` skill's own
  stated convention); adopting a nightly-only Cargo feature for a
  foundational, permanent piece of the build is a heavier commitment
  than the subprocess-and-parse workaround, which can be deleted with
  zero migration cost the day `-Zbindeps` stabilizes. `--target-dir`
  pointed at a fresh subdirectory of `OUT_DIR` (not the workspace's
  shared `target/`) avoids the worst of the lock-contention risk: the
  inner `cargo build` never touches the same lockfile-guarded directory
  the outer build is itself running inside.
- **Panics must never unwind across this FFI boundary — this is a new,
  Rust-specific hazard the existing C runtime structurally cannot have,
  and it gates every function this crate or any of its 96 planned
  successors ever exports.** Verified this session: `runtime/emerald_
  runtime.c`'s exception mechanism is `setjmp`/`longjmp`-based (plan 11's
  Decision log states plainly why: real DWARF-unwind-based exceptions
  were judged too large an undertaking at the time). `setjmp`/`longjmp`
  has no concept of Rust's own unwinding mechanism — a Rust panic that
  unwound past an `extern "C" fn` boundary into C frames using `setjmp`
  is undefined behavior twice over: once because unwinding across a
  plain (non-`"C-unwind"`) `extern "C"` boundary is already UB per
  Rust's own reference, and again because the C frames it would unwind
  through have no unwind tables describing them at all. Two ways to
  prevent this were weighed: workspace-wide `panic = "abort"` (simplest,
  but converts *any* bug anywhere in the eventual 96-domain stdlib
  surface — a malformed-input edge case three dependencies deep in an
  HTTP parser, say — into a hard, uncatchable process `abort()`, which
  would quietly break plan 59's own stated project identity: "ordinary
  Emerald code can never crash... in a way the type checker hasn't
  already ruled out" no longer holds once any native call can kill the
  whole process with no `rescue` able to intervene); or `std::panic::
  catch_unwind` wrapping the body of every single exported function,
  converting a caught panic into an ordinary Emerald-raised exception at
  the boundary (full design in plan 92; this plan's own `emerald_rt_
  fnv1a_hash` uses this pattern already, in working code, specifically
  so it is proven once here rather than only asserted in a later
  plan's prose). `catch_unwind` was chosen — `panic = "unwind"` stays
  the crate's real, effective panic strategy (recorded in `leaf-
  scaffold-emerald-rt-crate`), the opposite of what a Rust FFI-facing
  crate defaults to when its authors haven't thought about this
  specific boundary.
- **The first proof function is deliberately zero-dependency, not
  because a third-party crate would be unsafe, but so this plan's own
  correctness never depends on trusting one.** Every subsequent domain
  plan (96 onward) pulls in real, audited external crates — that trust
  decision belongs to plan 95's crate-vetting policy, applied per
  domain, not to this plan. FNV-1a is public-domain, roughly a dozen
  lines of arithmetic, and specified precisely enough that this plan's
  own hand-computed expected values and `emerald-rt`'s own `#[test]`
  assertion are independently checkable by any reader with no crate
  documentation to consult at all — the strongest possible proof that
  the *build and link mechanism* is what's being tested here, not any
  algorithm's correctness.
- **Out of scope.** No stdlib module, no error-code/`Result` marshaling
  convention beyond this one function's `-1`-on-panic placeholder (that
  is plan 92's job, generalized), no resource/handle lifetime model
  (plan 93 — this plan's proof function holds no resources at all), no
  async runtime of any kind (plan 94 — FNV-1a is synchronous CPU work,
  nothing to bridge), and no change to `emerald_runtime.c` itself — the
  C runtime is not being deprecated or migrated by this plan; it keeps
  every function it already has, unchanged, indefinitely. This plan
  adds a second archive; it does not begin retiring the first.

## Not yet decided (blocking EXECUTE)

1. The exact platform-library flag list (`-lpthread -ldl -lm` or a
   different/larger set) a `staticlib`-crate-type Rust archive needs at
   plain-`cc` link time on every target this project supports — stated
   as "confirm empirically" in `leaf-embed-and-link-second-archive`
   rather than asserted here, since it depends on exactly which Rust std
   features the eventual, much larger 96-domain stdlib surface pulls in,
   and guessing a complete list now for a crate that today calls nothing
   but `std::panic::catch_unwind` would be unverified speculation.
2. Whether the wasm32-wasip1 cross-compile path (`build.rs`'s existing
   gated second `cc::Build` call, per plan 64) gets a matching
   `emerald-rt`-for-wasm build in this plan or is deferred — `std::
   panic::catch_unwind` and thread-related std features some later
   domain plan will need may not all be available or meaningful under
   `wasm32-wasip1`'s single-execution-context model (mirroring the
   `emerald_runtime.c` file's own `#ifdef __wasi__` shims for exactly
   this reason). Deferring this to whichever domain plan first needs a
   thread/async-dependent crate under WASI, rather than deciding it
   speculatively here, is the more honest default.
