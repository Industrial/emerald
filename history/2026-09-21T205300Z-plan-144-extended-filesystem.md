2026-09-21T20:53:00Z

---
name: Extended Filesystem Operations — Directory Listing, Metadata, Symlinks
overview: "New `Dir`/`Path` compiler-known namespaces backed by `crates/emerald-rt` (plan 91's second, Rust-compiled static archive): `walkdir` for recursive directory traversal (a genuinely better-than-hand-rolled recursive descent — cycle-safe on symlinked loops, depth-limitable, actively maintained, used by ripgrep and ~cargo's own tooling) and plain `std::fs`/`std::os::unix::fs` for non-recursive listing, file metadata (size/modified-time/permissions), and symlink creation/reading — zero third-party dependency for that half, since `std::fs::Metadata`/`Permissions` already expose everything this plan needs natively. Strictly additive to plan 45's existing `File.read`/`File.write`: this plan never touches `runtime/emerald_runtime.c`, never changes those two functions' behavior, and covers only operations plan 45 explicitly never attempted — listing a directory's contents, walking a tree, and reading the metadata/symlink layer around a file, as opposed to plan 45's whole-file byte-slurp/byte-write."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-dir-namespace-and-walkdir-crate
    content: "Add `walkdir = \"2\"` to `crates/emerald-rt/Cargo.toml`. Implement `emerald_rt_dir_entries`/`emerald_rt_dir_entries_count` (non-recursive `std::fs::read_dir`) and `emerald_rt_dir_walk`/`emerald_rt_dir_walk_count` (recursive `walkdir::WalkDir::new(path).into_iter()`), each `#[no_mangle] extern \"C\"` and wrapped in `std::panic::catch_unwind` per plan 91's proven pattern, returning a heap-allocated array of null-terminated `char*` (mirroring `ValKind::Str`'s own representation, no new pointer convention invented) plus a matching `Int64` count function, exactly the `.split`/`.split_count` two-independent-call shape plan 45 already established for `Array[T]`'s no-length-metadata representation."
    status: pending
  - id: leaf-path-namespace-predicates-and-metadata
    content: "Add a new `Path` compiler-known namespace (`Expr::Ident(n) if n == \"Path\"`, the exact hard-coded-arm shape plan 45 used for `File` — never declared via a `ModuleDef`, so it can never collide with `local_classes`): `Path.exists/is_file/is_dir/is_symlink(path): Boolean` backed by `std::path::Path::{exists,is_file,is_dir,is_symlink}`, and `Path.metadata(path): FileMetadata?` — a new opaque built-in reference type (same inert, sema-only-distinction shape as plan 59's `CString`) returned nil on any `std::fs::metadata` error, with `.size(): Int64`, `.modified_unix(): Int64`, `.is_dir()/.is_file()/.readonly(): Boolean` as zero-argument method intrinsics dispatched off a new `ValKind::FileMetadata`, the same receiver-storage-kind mechanism plan 45 built for `ValKind::Str`."
    status: pending
  - id: leaf-unix-permissions-and-symlinks
    content: "`Path.unix_mode(path): Int64` (raw `st_mode` bits via `std::os::unix::fs::PermissionsExt::mode`, Unix-only — see Decision log), `Path.set_unix_mode(path, mode): Void` (`std::fs::set_permissions` + `PermissionsExt::from_mode`), `Path.symlink(target, link_path): Void` (`std::os::unix::fs::symlink`), `Path.read_link(path): String?` (`std::fs::read_link`, nil on error, reusing plan 43/59's zero-cost nullable-pointer convention)."
    status: pending
  - id: leaf-codegen-and-sema-wiring
    content: "Declare all ten new `emerald_rt_*` symbols in `emerald-codegen` via `module.add_function(..., Some(Linkage::External))`, the identical three-line shape plan 59 verified for the ~30 existing runtime declarations. Add the `Dir`/`Path` intrinsic arms to `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`, ordered before the generic `Type::Class` fallback arm purely for hygiene (never actually reachable — `Dir`/`Path` are never in `classes`, matching plan 45's own stated reasoning for `File`)."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/extended_filesystem_proof.em` (the Concrete Proof below) to `examples/` and wire it into `emerald-cli/tests/examples.rs`'s checked table. Add `#[test]`s inside `crates/emerald-rt` itself: one asserting `walkdir`-based `emerald_rt_dir_walk` visits every file in a fixture tree of nested directories (built with `tempfile::tempdir` — a forward reference to plan 147's crate, used here only as a test fixture, not a runtime dependency), one asserting `Path.metadata`'s size/is_dir fields against a real written file, one asserting `Path.symlink`+`Path.read_link` round-trips on a real symlink."
    status: pending
isProject: false
---

# Plan 144 — Extended Filesystem Operations

Plan 45 shipped exactly two filesystem primitives — `File.read(path): String`
and `File.write(path, content): Void` — a deliberate, disclosed minimum:
whole-file byte-slurp and byte-write, backed by two hand-written C functions
in `runtime/emerald_runtime.c`, with file I/O errors aborting the process
(`fprintf(stderr, ...); exit(1)`, matching `emerald_hash_key_not_found`'s
own precedent, per plan 45's Decision log). Plan 45 never touched directory
listing, recursive traversal, file metadata, permissions, or symlinks —
none of that is in its todos, its Concrete Proof, or its Decision log. This
plan fills exactly that gap, and only that gap. It changes nothing about
`File.read`/`File.write`: no new argument, no new error-handling behavior,
no touch to `runtime/emerald_runtime.c` at all. Every function this plan
adds is new Rust code in `crates/emerald-rt` (plan 91's second static
archive), declared as new `Dir`/`Path` compiler-known namespaces sitting
alongside `File`, not inside it — three sibling namespaces backed by two
different implementation layers (C for `File`, Rust for `Dir`/`Path`),
exactly the coexistence plan 91's own Decision log states explicitly: "This
plan adds a second archive; it does not begin retiring the first."

A real, disclosed divergence from plan 45's own error-handling precedent:
where `File.read`/`File.write` abort the process on failure, this plan's
`Dir`/`Path` functions surface failure as `nil` (a `String?`/`FileMetadata?`
return) wherever plan 43's nullable-reference convention already applies
cleanly, rather than aborting. This is possible specifically because this
is new code, not a fix to `emerald_runtime.c`'s existing abort behavior —
plan 92's general FFI error-marshaling convention is the load-bearing
mechanism underneath every one of this plan's fallible calls (every
`emerald_rt_*` export here is wrapped in `std::panic::catch_unwind` per
plan 91's proven pattern first; plan 92 owns the general shape of what
happens on the caught-panic path project-wide).

## Concrete proof this plan targets

```ruby
File.write("plan144_demo/a.txt", "hello")
File.write("plan144_demo/nested/b.txt", "world")

entries: Array[String] = Dir.entries("plan144_demo")
n: Int64 = Dir.entries_count("plan144_demo")
puts n

all: Array[String] = Dir.walk("plan144_demo")
m: Int64 = Dir.walk_count("plan144_demo")
puts m

meta: FileMetadata? = Path.metadata("plan144_demo/a.txt")
meta ||= FileMetadata.none
puts meta.size
puts meta.is_dir

Path.symlink("plan144_demo/a.txt", "plan144_demo/a_link.txt")
target: String? = Path.read_link("plan144_demo/a_link.txt")
target ||= "no link"
puts target
```

Expected output, in order (run from a fresh temporary working directory,
same discipline as plan 45's own Concrete Proof, so `plan144_demo/` is
freshly created):
```
2
3
5
false
plan144_demo/a.txt
```

Trace: `Dir.entries` on `plan144_demo` (non-recursive) sees `a.txt` and the
`nested` subdirectory itself — 2 entries, proving the non-recursive
`std::fs::read_dir` path. `Dir.walk` (recursive, `walkdir`) sees `a.txt`,
`nested`, and `nested/b.txt` — 3 entries, proving the recursive descent
`Dir.entries` does not attempt. `meta.size` prints `5` (the byte length of
`"hello"`, proving `Path.metadata` reads the same file `File.write` really
wrote) and `meta.is_dir` prints `false`. `Path.symlink`+`Path.read_link`
round-trips the real target path.

## Decision log

- **`walkdir` is chosen over hand-rolling recursive descent because it
  solves a genuinely non-trivial correctness problem this plan would
  otherwise have to solve itself: symlink cycles.** A directory tree
  containing a symlink back to one of its own ancestors turns a naive
  recursive `read_dir` into an infinite loop; `walkdir` tracks visited
  device/inode pairs (via `same_file`, its own stated dependency) to
  detect and skip these safely by default, and is the crate this
  ecosystem already converges on for exactly this job — `ripgrep`,
  `cargo`, and a wide field of other tools depend on it (verified this
  session: its own crates.io listing states it provides "an efficient and
  cross platform implementation of recursive directory traversal" with
  configurable symlink-following, depth limits, and sorting). Writing an
  equivalent cycle-safe walker from scratch for this plan alone would
  duplicate work a widely-audited crate already does correctly — exactly
  the "prefer a vetted crate over hand-rolled C" directive plan 91's own
  framing states, applied to its first real domain.
- **`Path`'s permission/symlink surface is a real, disclosed Unix-only
  limitation, not a cross-platform abstraction this plan pretends to
  build.** `std::os::unix::fs::PermissionsExt` (the `.mode()` accessor
  `Path.unix_mode` calls) and `std::os::unix::fs::symlink` are both
  gated behind `#[cfg(unix)]` in `std` itself — there is no equivalent
  single-call symlink constructor on Windows (`std::os::windows::fs`
  splits it into `symlink_file`/`symlink_dir`, a real API-shape
  difference, not just a naming one, since Windows distinguishes the two
  at creation time in a way Unix does not). This plan's `emerald-rt`
  implementation of `Path.unix_mode`/`Path.set_unix_mode`/`Path.symlink`
  is written against `#[cfg(unix)]` only; a Windows build of `emerald-rt`
  either stubs these three functions to a disclosed sentinel-and-panic
  (caught by the same `catch_unwind` boundary, surfacing as a normal
  Emerald exception rather than a link failure) or omits them from the
  Windows build entirely — this plan does not resolve which, deferred to
  `Not yet decided` below, since plan 91 itself has not yet decided
  whether `emerald-rt` gets built for any target beyond the primary Linux
  one this session's toolchain targets.
- **`FileMetadata` is a new opaque built-in reference type, not a
  user-visible class — the identical mechanism plan 59 already used for
  `CString`.** Plan 59's own Decision log states `CString` is "a new,
  deliberately inert reference type (sema-only distinction,
  `emerald-codegen` stores it identically to `ValKind::Str`... no
  `String` method dispatches on it at all)". `FileMetadata` follows this
  exactly: a new `ValKind::FileMetadata` tag over a small heap-allocated
  struct (four fields: size, modified-time, is_dir, is_file, readonly —
  copied out of `std::fs::Metadata` once, at the `Path.metadata` call
  site, never re-queried), with its own four zero-argument method
  intrinsics dispatched the same receiver-storage-kind way `.upcase`/
  `.length` already are. No `class FileMetadata ... end` is ever parsed
  from `.em` source — like `File`, it is never declared, never in
  `classes`, and cannot collide with a user type of the same name (the
  same disclosed narrow-namespace-collision non-issue plan 45's own
  Decision log already states for `File`).
- **No `Time` type exists anywhere in this compiler, so `.modified_unix`
  returns a bare `Int64` Unix-epoch-seconds value, not a richer temporal
  type.** `Type::String`'s own enumerated sibling list, verified directly
  against `emerald-sema`'s real `Type` enum this session via plan 59
  (`Int64, Float64, String, Boolean, Void, Symbol, Nil, Class, Array,
  Hash, Tuple, Enum, Result, Supervisor, Pair`), has no `Time`/`Instant`/
  `Duration` variant. Rather than inventing one as a side effect of this
  plan's four-field `FileMetadata`, `.modified_unix()` returns exactly
  what `std::time::SystemTime::duration_since(UNIX_EPOCH)` already gives:
  a signed 64-bit second count, the same representation `Int64` already
  is. A real `Time` type able to format, compare, and arithmetic on this
  value is a distinct, larger feature this plan declines to fold in.
- **`Array[T]`'s no-runtime-length-metadata wall, already documented by
  plan 45, is hit again here and fixed the identical way — a companion
  `_count` function, not a struct-returning ABI.** `Dir.entries`/
  `Dir.walk` both produce a runtime-determined number of `String`
  elements; plan 45's own Decision log already ruled out inventing a
  struct-returning calling convention to share one scan between a
  `.split`/`.split_count` pair, on the grounds that "every runtime call
  in `emerald_runtime.c` returns exactly one value" and a shared-scan ABI
  would be strictly more machinery than the plan needed. This plan is
  the second, independent confirmation that the two-scan workaround is
  the load-bearing pattern for *every* future `Array[T]`-returning
  intrinsic this whole 91-191 batch adds, not a one-off plan-45
  coincidence — `Dir.entries_count`/`Dir.walk_count` genuinely re-walk
  the directory a second time rather than caching the first walk's
  result, the same disclosed inefficiency plan 45 already accepted.
- **`FileMetadata` needs none of plan 93's explicit-close resource
  model.** It holds no OS handle, no open file descriptor, no lock —
  every field is a plain scalar copied out of `std::fs::Metadata` once,
  at construction time, in `Path.metadata`'s own Rust body, before the
  handle ever crosses back into Emerald. It is ordinary heap-allocated
  data under Emerald's existing arena/no-destructor model (the same "no
  free, no lifetime tracking" contract `emerald_alloc`'s own doc comment
  already discloses, per plan 45's citation of it) — not a resource in
  plan 93's sense at all. Plan 93 applies to this batch's genuinely
  stateful siblings — plan 148's held file lock, plan 149's watch
  subscription, plan 145's spawned child — not to this plan's inert
  metadata snapshot.
- **Out of scope.** Recursive copy/move/delete of a directory tree
  (`cp -r`/`rm -rf` equivalents — a real, larger feature needing its own
  error-accumulation story for partial failures mid-tree, not attempted
  here); hard links (`std::fs::hard_link` exists and would be a small
  addition, but this plan's four leaves are already the minimum needed
  to prove the `walkdir`+`std::fs` mechanism, and hard links serve a
  narrower, less commonly needed use case than symlinks); any change to
  `File.read`/`File.write` or `runtime/emerald_runtime.c` whatsoever;
  filesystem change *notification* (watching a directory for future
  events, as opposed to this plan's point-in-time snapshot) — that is
  plan 149, an entirely different mechanism (a long-lived OS watch
  subscription vs. this plan's one-shot syscalls); glob-pattern-filtered
  listing — that is plan 150, which is explicitly a thin, pattern-aware
  layer *on top of* the plain enumeration this plan provides (see plan
  150's own Decision log for the exact relationship).

## Not yet decided

1. Whether `crates/emerald-rt`'s Unix-only functions (`Path.unix_mode`,
   `Path.set_unix_mode`, `Path.symlink`) get a Windows stub (panicking
   through `catch_unwind` into a real Emerald exception) or are omitted
   from a hypothetical Windows build of `emerald-rt` entirely — blocked
   on plan 91's own still-open question of which targets `emerald-rt`
   builds for at all beyond this session's primary Linux target.
