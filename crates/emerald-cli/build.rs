//! Plan 27's runtime-bundling fix: compiles `runtime/emerald_runtime.c`
//! into a static archive at `emerald-cli`'s own build time (via the
//! `cc` crate — the standard way to invoke a C compiler from a build
//! script), so the *shipped binary* no longer needs this repo's
//! `runtime/emerald_runtime.c` present at link-a-user's-program time.
//!
//! Emitting the archive's on-disk `OUT_DIR` path alone would not
//! actually fix portability — `OUT_DIR` lives inside this repo's own
//! `target/` tree and doesn't exist once the binary is copied
//! elsewhere. Instead this exposes the path via `EMERALD_RUNTIME_ARCHIVE`
//! so `main.rs` can `include_bytes!` it: that embeds the archive's real
//! bytes into the compiled `emerald-cli` binary itself, at compile
//! time, which is what makes a copied-alone binary still able to link
//! a user's program with no access to this checkout at all.

fn main() {
  let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
  let runtime_src = std::path::Path::new(&manifest_dir)
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist relative to this crate's manifest dir");

  println!("cargo:rerun-if-changed={}", runtime_src.display());

  // `cc::Build::compile("emerald_runtime")` produces
  // `{OUT_DIR}/libemerald_runtime.a` by its own documented convention.
  cc::Build::new()
    .file(&runtime_src)
    .compile("emerald_runtime");

  let out_dir = std::env::var("OUT_DIR").expect("set by cargo");
  let archive_path = std::path::Path::new(&out_dir).join("libemerald_runtime.a");
  println!(
    "cargo:rustc-env=EMERALD_RUNTIME_ARCHIVE={}",
    archive_path.display()
  );
}
