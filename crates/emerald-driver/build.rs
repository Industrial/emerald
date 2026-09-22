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

  build_emerald_rt_archive(&manifest_dir, &out_dir);
}

/// Plan 91's `leaf-json-artifact-discovery-in-build-rs`: builds
/// `crates/emerald-rt` as a real `cargo build -p emerald-rt --release`
/// subprocess and discovers its `staticlib` artifact's path by parsing
/// `--message-format=json`'s NDJSON stream — stable Cargo has no
/// artifact-dependency feature (`-Zbindeps` is nightly-only, and this
/// project's toolchain is pinned to a stable release per the
/// `rust-devenv` skill's own convention) to obtain a sibling crate's
/// compiled staticlib path any other way. `--target-dir` is pointed at
/// a fresh subdirectory of this crate's own `OUT_DIR`, never the
/// workspace's shared `target/`, specifically so this inner `cargo`
/// invocation never contends for the same lockfile-guarded directory
/// the outer build is itself already running inside — recursive
/// `cargo build` from a build script is a real, named anti-pattern
/// (lock contention, environment-variable leakage) adopted here
/// deliberately, with that one mitigation, not accidentally.
fn build_emerald_rt_archive(manifest_dir: &str, out_dir: &str) {
  let emerald_rt_src = std::path::Path::new(manifest_dir)
    .join("../emerald-rt/src/lib.rs")
    .canonicalize()
    .expect("crates/emerald-rt/src/lib.rs must exist relative to this crate's manifest dir");
  println!("cargo:rerun-if-changed={}", emerald_rt_src.display());
  let emerald_rt_manifest = std::path::Path::new(manifest_dir)
    .join("../emerald-rt/Cargo.toml")
    .canonicalize()
    .expect("crates/emerald-rt/Cargo.toml must exist relative to this crate's manifest dir");
  println!("cargo:rerun-if-changed={}", emerald_rt_manifest.display());

  let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
  let rt_target_dir = std::path::Path::new(out_dir).join("emerald-rt-target");
  let output = std::process::Command::new(&cargo)
    .args([
      "build",
      "--message-format=json",
      "--release",
      "--manifest-path",
    ])
    .arg(&emerald_rt_manifest)
    .arg("--target-dir")
    .arg(&rt_target_dir)
    .output()
    .expect("failed to invoke `cargo build -p emerald-rt` from build.rs");

  assert!(
    output.status.success(),
    "cargo build -p emerald-rt failed:\nstdout:\n{}\nstderr:\n{}",
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr)
  );

  // Each line of `--message-format=json`'s stdout is one independent
  // JSON object (Cargo's own documented NDJSON contract) — a line this
  // crate doesn't recognize (a `"build-finished"` message, a plain
  // compiler warning) is skipped, not an error; only the one
  // `"compiler-artifact"` message naming `emerald_rt`'s `staticlib`
  // target is real signal here.
  let mut archive_path: Option<String> = None;
  for line in String::from_utf8_lossy(&output.stdout).lines() {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
      continue;
    };
    if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-artifact") {
      continue;
    }
    let Some(target) = msg.get("target") else {
      continue;
    };
    // Cargo's own artifact messages report the TARGET name with
    // hyphens already replaced by underscores (`emerald-rt`'s package
    // name becomes `emerald_rt` here) — matched literally, not
    // normalized, since that is the real, stable contract this
    // message shape has always used.
    if target.get("name").and_then(|n| n.as_str()) != Some("emerald_rt") {
      continue;
    }
    let is_staticlib = target
      .get("kind")
      .and_then(|k| k.as_array())
      .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("staticlib")));
    if !is_staticlib {
      continue;
    }
    if let Some(filenames) = msg.get("filenames").and_then(|f| f.as_array()) {
      archive_path = filenames
        .iter()
        .filter_map(|f| f.as_str())
        .find(|f| f.ends_with(".a"))
        .map(str::to_string);
    }
  }

  let archive_path = archive_path.expect(
    "cargo build -p emerald-rt --message-format=json produced no compiler-artifact message \
     naming an `emerald_rt` staticlib — its own stdout is printed above on failure",
  );
  println!("cargo:rustc-env=EMERALD_RT_ARCHIVE={archive_path}");
}
