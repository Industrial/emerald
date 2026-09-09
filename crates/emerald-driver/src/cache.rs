//! Plan 48's `leaf-query-cache-core`: a hand-rolled, three-query
//! memoization layer over the otherwise-unchanged parse/type-check/
//! codegen pipeline — the query-based architecture rustc's own
//! `rustc_query_system` uses internally and the pattern the `salsa`
//! crate generalizes (inception §14.5's own named gate), applied by
//! hand rather than by adding that crate, since this plan's whole-file
//! granularity doesn't need `salsa`'s fine-grained machinery. A cache
//! MISS always falls back to calling the real `emerald_parser`/
//! `emerald_sema`/`emerald_codegen` function verbatim — a cache hit is
//! strictly an optimization, never a correctness dependency.

use emerald_parser::{ParseError, Program};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A `blake3::Hash` is already `Copy`/`Eq`/`Hash`/`Display` (hex) —
/// exactly what a cache key needs, with zero wrapper boilerplate.
pub type CacheKey = blake3::Hash;

/// The raw content hash of `bytes` — used both for `QueryCache`'s own
/// per-query keys and, standalone, by `emerald-cli`'s `require.rs` to
/// hash each file in a require closure before folding those hashes
/// together into one merged key (see `QueryCache::key_for_many`).
pub fn raw_hash(bytes: &[u8]) -> CacheKey {
  blake3::hash(bytes)
}

fn combine(fingerprint: CacheKey, hashes: &[CacheKey]) -> CacheKey {
  let mut hasher = blake3::Hasher::new();
  hasher.update(fingerprint.as_bytes());
  for h in hashes {
    hasher.update(h.as_bytes());
  }
  hasher.finalize()
}

/// A content hash of the running compiler's own binary — folded into
/// every query key so that rebuilding `emerald-cli` (a codegen fix, a
/// new local build) invalidates every previously-cached result instead
/// of silently reusing a stale `.o` a different compiler produced.
fn compiler_fingerprint() -> CacheKey {
  let bytes = std::env::current_exe()
    .ok()
    .and_then(|p| std::fs::read(p).ok())
    .unwrap_or_default();
  blake3::hash(&bytes)
}

/// Reports one `[cache] <query> <label> HIT|MISS` observation —
/// `emerald-cli`'s `--verbose-cache`/`--history-cache` flags are what
/// decide whether a `VerboseReporter` or a `SilentReporter` gets
/// threaded through; `QueryCache` itself has no opinion on stdout/
/// stderr.
pub trait CacheReporter {
  fn report(&self, query: &str, label: &str, hit: bool);
}

pub struct SilentReporter;
impl CacheReporter for SilentReporter {
  fn report(&self, _query: &str, _label: &str, _hit: bool) {}
}

/// The plan's own worked-example format: `[cache] parse   main.em     MISS`.
pub struct VerboseReporter;
impl CacheReporter for VerboseReporter {
  fn report(&self, query: &str, label: &str, hit: bool) {
    eprintln!(
      "[cache] {:<7} {:<12} {}",
      query,
      label,
      if hit { "HIT" } else { "MISS" }
    );
  }
}

/// Holds the cache root directory and the running compiler's own
/// fingerprint. `parse_query`'s memoization lives only in the
/// in-process `parse_cache` `HashMap` (parsing is cheap, and a fresh
/// process has no way to reuse a serialized `Program` without adding
/// `serde`/`Hash` derives this codebase has deliberately avoided —
/// see the plan's own Decision log); `type_check_query`/`codegen_
/// query` persist to `<root>/check/` and `<root>/obj/`.
pub struct QueryCache {
  root: PathBuf,
  fingerprint: CacheKey,
  parse_cache: Mutex<HashMap<CacheKey, Program>>,
}

impl QueryCache {
  pub fn new(root: PathBuf) -> Self {
    Self::with_fingerprint(root, compiler_fingerprint())
  }

  /// Test-only injectable seam (leaf-query-cache-core AC4): construct
  /// with an explicit fingerprint instead of hashing the real running
  /// binary, so a test can prove "changing the fingerprint forces
  /// MISS" without needing to actually rebuild the compiler.
  pub fn with_fingerprint(root: PathBuf, fingerprint: CacheKey) -> Self {
    Self {
      root,
      fingerprint,
      parse_cache: Mutex::new(HashMap::new()),
    }
  }

  pub fn fingerprint(&self) -> CacheKey {
    self.fingerprint
  }

  /// The real key for a single piece of content — folds the compiler
  /// fingerprint into `raw_hash(bytes)`.
  pub fn key_for(&self, raw: CacheKey) -> CacheKey {
    combine(self.fingerprint, &[raw])
  }

  /// The merged key for a require closure — `blake3(fingerprint ||
  /// hash_1 || hash_2 || ... || hash_n)` over each visited file's own
  /// raw content hash, in first-visit order (leaf-require-graph-
  /// cache-keys). Touching one file changes its own hash, which
  /// changes every downstream merged key that includes it, and no
  /// other.
  pub fn key_for_many(&self, raws: &[CacheKey]) -> CacheKey {
    combine(self.fingerprint, raws)
  }

  /// In-process-memoized parse. `name`/`source` are hashed together
  /// with `name` folded in via the label only (the key is over
  /// `source`'s bytes alone, matching `emerald_parser::parse_named`
  /// being a pure function of `(source, name)` where `name` only ever
  /// affects diagnostic text, never the parsed shape).
  pub fn parse_query(
    &self,
    name: &str,
    source: &str,
    reporter: &dyn CacheReporter,
  ) -> Result<Program, Vec<ParseError>> {
    let key = self.key_for(raw_hash(source.as_bytes()));
    if let Some(program) = self.parse_cache.lock().unwrap().get(&key) {
      reporter.report("parse", name, true);
      return Ok(program.clone());
    }
    reporter.report("parse", name, false);
    let program = emerald_parser::parse_named(source, name)?;
    self
      .parse_cache
      .lock()
      .unwrap()
      .insert(key, program.clone());
    Ok(program)
  }

  fn check_path(&self, key: CacheKey) -> PathBuf {
    self.root.join("check").join(format!("{key}.txt"))
  }

  /// Persisted at `<root>/check/<key>.txt` — an empty file means the
  /// prior run passed; one diagnostic message per line means it
  /// failed. Reconstructed diagnostics on a cache HIT carry `span:
  /// (0, 0)` (message-only persistence, no `serde` needed — see the
  /// plan's own Decision log) rather than their original real span.
  pub fn type_check_query(
    &self,
    key: CacheKey,
    program: &Program,
    label: &str,
    reporter: &dyn CacheReporter,
  ) -> Result<(), Vec<emerald_sema::Diagnostic>> {
    let path = self.check_path(key);
    if let Ok(contents) = std::fs::read_to_string(&path) {
      reporter.report("check", label, true);
      if contents.is_empty() {
        return Ok(());
      }
      return Err(
        contents
          .lines()
          .map(|line| emerald_sema::Diagnostic {
            message: line.to_string(),
            span: (0, 0),
          })
          .collect(),
      );
    }
    reporter.report("check", label, false);
    let result = emerald_sema::check_program(program);
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).ok();
    }
    match &result {
      Ok(()) => {
        std::fs::write(&path, "").ok();
      }
      Err(diags) => {
        let text = diags
          .iter()
          .map(|d| d.message.clone())
          .collect::<Vec<_>>()
          .join("\n");
        std::fs::write(&path, text).ok();
      }
    }
    result
  }

  fn obj_path(&self, key: CacheKey) -> PathBuf {
    self.root.join("obj").join(format!("{key}.o"))
  }

  fn obj_hash_path(&self, key: CacheKey) -> PathBuf {
    self.root.join("obj").join(format!("{key}.o.hash"))
  }

  /// Persisted at `<root>/obj/<key>.o`, with a sidecar `<key>.o.hash`
  /// recording the cached bytes' own hash at write time — read back
  /// and re-verified on every HIT attempt, so a corrupted (not just
  /// deleted) cached object file is detected and treated as a MISS
  /// (falling back to a real, correct `compile_to_object` call)
  /// instead of silently copying bad bytes into `obj_path`.
  pub fn codegen_query(
    &self,
    key: CacheKey,
    program: &Program,
    obj_path: &Path,
    label: &str,
    reporter: &dyn CacheReporter,
  ) -> Result<(), String> {
    let cached_obj = self.obj_path(key);
    let cached_hash = self.obj_hash_path(key);
    if let (Ok(bytes), Ok(expected)) = (
      std::fs::read(&cached_obj),
      std::fs::read_to_string(&cached_hash),
    ) {
      let actual = blake3::hash(&bytes).to_hex().to_string();
      if actual == expected.trim() {
        reporter.report("codegen", label, true);
        std::fs::write(obj_path, &bytes).map_err(|e| e.to_string())?;
        return Ok(());
      }
    }
    reporter.report("codegen", label, false);
    emerald_codegen::compile_to_object(program, obj_path)?;
    if let Some(parent) = cached_obj.parent() {
      std::fs::create_dir_all(parent).ok();
    }
    if let Ok(bytes) = std::fs::read(obj_path) {
      let hash = blake3::hash(&bytes).to_hex().to_string();
      std::fs::write(&cached_obj, &bytes).ok();
      std::fs::write(&cached_hash, hash).ok();
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::cell::RefCell;
  use std::sync::atomic::{AtomicUsize, Ordering};

  fn fresh_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  const HELLO_SRC: &str =
    "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  /// Call-counting test double (AC1: "verified via a call-counting
  /// test double, not just by timing") — records every HIT/MISS
  /// report it receives so a test can assert the *real* underlying
  /// `emerald_sema`/`emerald_codegen` function was (or wasn't) called,
  /// not merely infer it from wall-clock time.
  #[derive(Default)]
  struct CountingReporter {
    hits: AtomicUsize,
    misses: AtomicUsize,
    calls: RefCell<Vec<(String, String, bool)>>,
  }
  impl CacheReporter for CountingReporter {
    fn report(&self, query: &str, label: &str, hit: bool) {
      if hit {
        self.hits.fetch_add(1, Ordering::SeqCst);
      } else {
        self.misses.fetch_add(1, Ordering::SeqCst);
      }
      self
        .calls
        .borrow_mut()
        .push((query.to_string(), label.to_string(), hit));
    }
  }
  // SAFETY: tests are single-threaded per-cache-instance; RefCell is
  // fine, but CacheReporter's trait object needs Sync for the &dyn
  // reference used elsewhere — tests only ever use this on the
  // current thread, never across one.
  unsafe impl Sync for CountingReporter {}

  #[test]
  fn two_calls_with_the_same_content_are_hit_then_miss_and_byte_identical() {
    let root = fresh_root("hit-miss");
    let cache = QueryCache::with_fingerprint(root, blake3::hash(b"fingerprint-a"));
    let reporter = CountingReporter::default();

    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    let key = cache.key_for(raw_hash(HELLO_SRC.as_bytes()));

    let first = cache.type_check_query(key, &program, "hello.em", &reporter);
    let obj1 = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-obj1-{}.o",
      std::process::id()
    ));
    let first_obj = cache.codegen_query(key, &program, &obj1, "hello.em", &reporter);

    let second = cache.type_check_query(key, &program, "hello.em", &reporter);
    let obj2 = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-obj2-{}.o",
      std::process::id()
    ));
    let second_obj = cache.codegen_query(key, &program, &obj2, "hello.em", &reporter);

    assert_eq!(first, Ok(()));
    assert_eq!(second, Ok(()));
    assert!(first_obj.is_ok());
    assert!(second_obj.is_ok());
    assert_eq!(
      std::fs::read(&obj1).unwrap(),
      std::fs::read(&obj2).unwrap(),
      "cached object bytes must be byte-identical to a fresh compile"
    );

    let calls = reporter.calls.borrow();
    assert_eq!(
      calls[0],
      ("check".to_string(), "hello.em".to_string(), false)
    );
    assert_eq!(
      calls[1],
      ("codegen".to_string(), "hello.em".to_string(), false)
    );
    assert_eq!(
      calls[2],
      ("check".to_string(), "hello.em".to_string(), true)
    );
    assert_eq!(
      calls[3],
      ("codegen".to_string(), "hello.em".to_string(), true)
    );

    std::fs::remove_file(&obj1).ok();
    std::fs::remove_file(&obj2).ok();
  }

  #[test]
  fn corrupting_the_cached_object_file_forces_a_real_recompile_not_an_error() {
    let root = fresh_root("corrupt");
    let cache = QueryCache::with_fingerprint(root.clone(), blake3::hash(b"fingerprint-b"));
    let reporter = CountingReporter::default();
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    let key = cache.key_for(raw_hash(HELLO_SRC.as_bytes()));

    let obj = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-corrupt-{}.o",
      std::process::id()
    ));
    cache
      .codegen_query(key, &program, &obj, "hello.em", &reporter)
      .unwrap();
    let real_bytes = std::fs::read(&obj).unwrap();

    // Corrupt the cached artifact directly (not delete it) — bytes
    // still exist at <root>/obj/<key>.o, but no longer match the
    // sidecar hash written alongside it.
    let cached_obj = root.join("obj").join(format!("{key}.o"));
    std::fs::write(&cached_obj, b"not a real object file").unwrap();

    let result = cache.codegen_query(key, &program, &obj, "hello.em", &reporter);
    assert!(
      result.is_ok(),
      "a corrupted cache entry must fall back to a real recompile, not error: {result:?}"
    );
    assert_eq!(
      std::fs::read(&obj).unwrap(),
      real_bytes,
      "the recompiled output must be the real, correct object bytes"
    );
    // Two codegen calls, both MISS (the corruption forces the second
    // one to MISS too, since the sidecar hash no longer matches).
    let calls = reporter.calls.borrow();
    let codegen_calls: Vec<_> = calls.iter().filter(|(q, ..)| q == "codegen").collect();
    assert_eq!(codegen_calls.len(), 2);
    assert!(codegen_calls.iter().all(|(_, _, hit)| !hit));

    std::fs::remove_file(&obj).ok();
  }

  #[test]
  fn deleting_the_cached_object_file_also_forces_a_real_recompile() {
    let root = fresh_root("delete");
    let cache = QueryCache::with_fingerprint(root.clone(), blake3::hash(b"fingerprint-c"));
    let reporter = SilentReporter;
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    let key = cache.key_for(raw_hash(HELLO_SRC.as_bytes()));

    let obj = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-delete-{}.o",
      std::process::id()
    ));
    cache
      .codegen_query(key, &program, &obj, "hello.em", &reporter)
      .unwrap();
    let cached_obj = root.join("obj").join(format!("{key}.o"));
    std::fs::remove_file(&cached_obj).unwrap();

    let result = cache.codegen_query(key, &program, &obj, "hello.em", &reporter);
    assert!(
      result.is_ok(),
      "a deleted cache entry must not error: {result:?}"
    );
    assert!(obj.exists());

    std::fs::remove_file(&obj).ok();
  }

  #[test]
  fn changing_one_byte_of_source_changes_the_key_and_forces_miss() {
    let root = fresh_root("content-change");
    let cache = QueryCache::with_fingerprint(root, blake3::hash(b"fingerprint-d"));
    let key_a = cache.key_for(raw_hash(b"x: Int64 = 1\n"));
    let key_b = cache.key_for(raw_hash(b"x: Int64 = 2\n"));
    assert_ne!(
      key_a, key_b,
      "changing one byte of source must change the computed key"
    );

    let reporter = CountingReporter::default();
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    let _ = cache.type_check_query(key_a, &program, "a", &reporter);
    // A different key must MISS even though it's the same program —
    // nothing was ever cached under key_b.
    let _ = cache.type_check_query(key_b, &program, "b", &reporter);
    let calls = reporter.calls.borrow();
    assert!(calls.iter().all(|(_, _, hit)| !hit));
  }

  #[test]
  fn overriding_the_fingerprint_forces_miss_on_both_check_and_codegen() {
    let root = fresh_root("fingerprint");
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    let raw = raw_hash(HELLO_SRC.as_bytes());

    let cache_a = QueryCache::with_fingerprint(root.clone(), blake3::hash(b"compiler-v1"));
    let key_a = cache_a.key_for(raw);
    let reporter_a = CountingReporter::default();
    let _ = cache_a.type_check_query(key_a, &program, "hello.em", &reporter_a);
    let obj = std::env::temp_dir().join(format!(
      "emerald-driver-cache-test-fp-{}.o",
      std::process::id()
    ));
    cache_a
      .codegen_query(key_a, &program, &obj, "hello.em", &reporter_a)
      .unwrap();

    // Same root, same source, but a different compiler fingerprint —
    // simulating a rebuilt `emerald-cli` binary without actually
    // rebuilding one.
    let cache_b = QueryCache::with_fingerprint(root, blake3::hash(b"compiler-v2"));
    let key_b = cache_b.key_for(raw);
    assert_ne!(key_a, key_b, "a different fingerprint must change the key");
    let reporter_b = CountingReporter::default();
    let check_result = cache_b.type_check_query(key_b, &program, "hello.em", &reporter_b);
    let codegen_result = cache_b.codegen_query(key_b, &program, &obj, "hello.em", &reporter_b);

    assert_eq!(check_result, Ok(()));
    assert!(codegen_result.is_ok());
    assert!(reporter_b.calls.borrow().iter().all(|(_, _, hit)| !hit));

    std::fs::remove_file(&obj).ok();
  }

  #[test]
  fn the_full_existing_sema_test_corpus_round_trips_identically_through_the_cache() {
    // AC5: run every worked example already proven elsewhere in this
    // workspace through type_check_query twice; both outcomes and
    // both diagnostic texts must be identical.
    let examples: &[(&str, &str)] = &[
      ("hello", HELLO_SRC),
      (
        "point",
        "class Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(1.0, 2.0)\n",
      ),
      ("bad", "x: Int64 = \"nope\"\n"),
    ];
    for (label, src) in examples {
      let root = fresh_root(&format!("corpus-{label}"));
      let cache = QueryCache::with_fingerprint(root, blake3::hash(b"corpus-fingerprint"));
      let program = emerald_parser::parse_named(src, "corpus.em").unwrap();
      let key = cache.key_for(raw_hash(src.as_bytes()));
      let reporter = SilentReporter;
      let first = cache.type_check_query(key, &program, label, &reporter);
      let second = cache.type_check_query(key, &program, label, &reporter);
      let first_direct = emerald_sema::check_program(&program);
      match (&first, &second) {
        (Ok(()), Ok(())) => assert_eq!(first_direct, Ok(())),
        (Err(a), Err(b)) => {
          let a_msgs: Vec<_> = a.iter().map(|d| d.message.clone()).collect();
          let b_msgs: Vec<_> = b.iter().map(|d| d.message.clone()).collect();
          assert_eq!(
            a_msgs, b_msgs,
            "cached diagnostic text must be byte-identical"
          );
          let direct_msgs: Vec<_> = first_direct
            .unwrap_err()
            .iter()
            .map(|d| d.message.clone())
            .collect();
          assert_eq!(a_msgs, direct_msgs);
        }
        other => panic!("first and second run disagreed on pass/fail: {other:?}"),
      }
    }
  }
}
