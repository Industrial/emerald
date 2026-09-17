//! Plan 27's runtime-bundling fix, moved here unchanged from
//! `emerald-cli` (plan 17's `leaf-driver-extraction`): compiles
//! `runtime/emerald_runtime.c` into a static archive at this crate's
//! own build time (via the `cc` crate), so the *shipped binary*
//! (`emerald-cli`, or any future caller linking through this driver)
//! no longer needs this repo's `runtime/emerald_runtime.c` present at
//! link-a-user's-program time.
//!
//! Emitting the archive's on-disk `OUT_DIR` path alone would not
//! actually fix portability — `OUT_DIR` lives inside this repo's own
//! `target/` tree and doesn't exist once the binary is copied
//! elsewhere. Instead this exposes the path via `EMERALD_RUNTIME_ARCHIVE`
//! so `lib.rs` can `include_bytes!` it: that embeds the archive's real
//! bytes into the compiled binary itself, at compile time, which is
//! what makes a copied-alone binary still able to link a user's
//! program with no access to this checkout at all.
//!
//! `crates/emerald-driver` sits at the same depth under `crates/` as
//! `crates/emerald-cli` did, so the relative path to `runtime/
//! emerald_runtime.c` is unchanged.

fn main() {
  let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
  let runtime_src = std::path::Path::new(&manifest_dir)
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist relative to this crate's manifest dir");

  println!("cargo:rerun-if-changed={}", runtime_src.display());

  // Bugfix (benchmark session): `emerald_runtime.c` is one translation
  // unit compiled to one `.o` inside the archive — by default the
  // linker can only pull in a static-archive member whole-or-nothing,
  // so any single referenced symbol (even just `emerald_print_i64` for
  // a bare `puts`) drags in every actor/networking/supervision function
  // too, regardless of whether the compiled program uses them. Measured
  // this session: every benchmark's Emerald binary landed in the same
  // flat ~90 KB band no matter what the program actually did. `-ffunction-
  // sections`/`-fdata-sections` here move every function/global into its
  // own linker section instead of one per-file blob, so `--gc-sections`
  // at the link step (`build_link_args`/`link_many`, `emerald-driver/
  // src/lib.rs`/`src/parallel.rs`) can prune unreferenced ones — the
  // standard fix for this exact "one translation unit forces monolithic
  // linking" shape, not a novel technique.
  cc::Build::new()
    .file(&runtime_src)
    .flag("-ffunction-sections")
    .flag("-fdata-sections")
    .compile("emerald_runtime");

  let out_dir = std::env::var("OUT_DIR").expect("set by cargo");
  let archive_path = std::path::Path::new(&out_dir).join("libemerald_runtime.a");
  println!(
    "cargo:rustc-env=EMERALD_RUNTIME_ARCHIVE={}",
    archive_path.display()
  );

  // Plan 64's `leaf-wasi-runtime-and-sequential-actors`: a second,
  // gated cross-compile of the SAME runtime source for `wasm32-wasip1`
  // — attempted only when a WASI-capable compiler is actually
  // configured via `CC_wasm32_wasip1` (the same underscored env-var
  // spelling the `cc` crate's own documented cross-compilation
  // convention reads for a `.target("wasm32-wasip1")` build). Building
  // this crate on a machine with no WASI toolchain at all (this
  // workspace's own `devenv shell`, verified this session — no
  // `wasi-sdk`/`WASI_SDK_PATH`/`wasmtime` present) must never fail
  // `cargo build -p emerald-driver` just because nobody asked for WASM
  // support — a real, empty placeholder archive is written instead,
  // and `link_with_libs`'s own `Wasm32Wasi` branch (`src/lib.rs`)
  // checks for exactly this emptiness at run time, returning a real,
  // named `DriverError::Link("no WASI toolchain configured...")`
  // rather than trying to link a garbage/empty archive.
  println!("cargo:rerun-if-env-changed=CC_wasm32_wasip1");
  let wasm_archive_path = std::path::Path::new(&out_dir).join("libemerald_runtime_wasm32_wasi.a");
  if std::env::var("CC_wasm32_wasip1").is_ok() {
    cc::Build::new()
      .file(&runtime_src)
      .target("wasm32-wasip1")
      .opt_level(2)
      .compile("emerald_runtime_wasm32_wasi");
  } else {
    std::fs::write(&wasm_archive_path, []).expect("should write an empty WASI-archive placeholder");
  }
  println!(
    "cargo:rustc-env=EMERALD_RUNTIME_ARCHIVE_WASM32_WASI={}",
    wasm_archive_path.display()
  );
}
