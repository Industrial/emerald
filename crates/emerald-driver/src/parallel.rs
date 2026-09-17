//! Plan 49's `leaf-parallel-parse-and-typecheck`/
//! `leaf-parallel-codegen-and-jobs-flag`: schedules the already-
//! independent per-file query invocations `require_graph`/plan 48's
//! `QueryCache` provide onto a bounded `std::thread::scope` worker
//! pool — parsing is fully free (level-independent, every file's own
//! bytes are all it needs); type-checking and codegen are leveled
//! (Kahn's algorithm order from `require_graph::compute_levels`), with
//! a hard barrier between levels since a file may reference symbols a
//! dependency exports. No `rayon` — see the plan's own Decision log:
//! a `Mutex<VecDeque<PathBuf>>` drained by a fixed number of
//! `std::thread::scope`-spawned workers is the whole scheduler this
//! plan needs.

use crate::cache::{CacheReporter, QueryCache};
use crate::require_graph::{
  RequireGraph, build_require_graph, closure_hashes, closure_items, compute_levels,
  own_function_names, unsupported_construct,
};
use crate::{DriverError, RUNTIME_ARCHIVE};
use emerald_parser::Program;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::Mutex;
use std::time::Instant;

/// One `(file, thread, start, end)` record — populated only when
/// `EMERALD_TRACE_COMPILE=1` (leaf-parallel-codegen-and-jobs-flag's
/// own timed concurrency proof seam), dumped as newline-delimited
/// `file\tthread_id\tstart_nanos\tend_nanos` records to the path named
/// by `EMERALD_TRACE_COMPILE_OUT` once every level's codegen finishes.
struct TraceRecord {
  file: PathBuf,
  thread_id: String,
  start_nanos: u128,
  end_nanos: u128,
}

/// The full parallel pipeline: build the require graph, level it,
/// parse every file (fully in parallel), type-check and codegen level
/// by level (parallel within a level, a hard barrier between levels),
/// then link every level's own object file into one binary.
pub fn compile_parallel(
  entry: &Path,
  output_path: &Path,
  jobs: usize,
  cache: &QueryCache,
  // Plan 49: narrower than plan 48's own `&dyn CacheReporter` — this
  // module shares `reporter` across `std::thread::scope` worker
  // threads, which needs `Sync`. `SilentReporter`/`VerboseReporter`
  // (this crate's own stateless implementors, the only ones a
  // multi-file `--jobs` compile ever constructs) already satisfy this
  // for free; `cache.rs`'s own `CountingReporter` test double (which
  // uses a `RefCell` and is deliberately single-threaded) is
  // untouched, since the plain `CacheReporter` trait itself keeps no
  // `Sync` bound.
  reporter: &(dyn CacheReporter + Sync),
) -> Result<(), DriverError> {
  let jobs = jobs.max(1);
  let graph = build_require_graph(entry)?;

  // Fail fast, before any parallel work starts, on a construct this
  // pass's per-file codegen split doesn't support yet (see
  // `require_graph::unsupported_construct`'s own doc comment).
  let mut unsupported: Vec<String> = graph
    .nodes
    .keys()
    .filter_map(|p| unsupported_construct(&graph, p))
    .collect();
  unsupported.sort();
  if let Some(first) = unsupported.into_iter().next() {
    return Err(DriverError::Require(first));
  }

  let levels = compute_levels(&graph);

  parallel_parse_all(&graph, jobs, cache, reporter)?;
  for level in &levels {
    parallel_type_check_level(&graph, level, jobs, cache, reporter)?;
  }

  let trace_enabled = std::env::var("EMERALD_TRACE_COMPILE").as_deref() == Ok("1");
  let trace: Mutex<Vec<TraceRecord>> = Mutex::new(Vec::new());
  let mut obj_paths: Vec<(PathBuf, PathBuf)> = Vec::new();
  for level in &levels {
    let mut level_objs = parallel_codegen_level(&graph, level, jobs, cache, reporter, &trace)?;
    obj_paths.append(&mut level_objs);
  }
  if trace_enabled {
    dump_trace(&trace);
  }
  obj_paths.sort_by(|a, b| a.0.cmp(&b.0));
  let objs: Vec<PathBuf> = obj_paths.into_iter().map(|(_, o)| o).collect();

  link_many(objs, output_path.to_path_buf())
}

fn parallel_parse_all(
  graph: &RequireGraph,
  jobs: usize,
  cache: &QueryCache,
  // Plan 49: narrower than plan 48's own `&dyn CacheReporter` — this
  // module shares `reporter` across `std::thread::scope` worker
  // threads, which needs `Sync`. `SilentReporter`/`VerboseReporter`
  // (this crate's own stateless implementors, the only ones a
  // multi-file `--jobs` compile ever constructs) already satisfy this
  // for free; `cache.rs`'s own `CountingReporter` test double (which
  // uses a `RefCell` and is deliberately single-threaded) is
  // untouched, since the plain `CacheReporter` trait itself keeps no
  // `Sync` bound.
  reporter: &(dyn CacheReporter + Sync),
) -> Result<(), DriverError> {
  let queue: Mutex<VecDeque<&PathBuf>> = Mutex::new(graph.nodes.keys().collect());
  let errors: Mutex<Vec<(PathBuf, Vec<emerald_parser::ParseError>)>> = Mutex::new(Vec::new());

  std::thread::scope(|scope| {
    for _ in 0..jobs {
      scope.spawn(|| {
        loop {
          let path = { queue.lock().unwrap().pop_front() };
          let Some(path) = path else { break };
          let node = &graph.nodes[path];
          let name = path.to_string_lossy().to_string();
          if let Err(errs) = cache.parse_query(&name, &node.source, reporter) {
            errors.lock().unwrap().push((path.clone(), errs));
          }
        }
      });
    }
  });

  let mut errors = errors.into_inner().unwrap();
  if errors.is_empty() {
    return Ok(());
  }
  // Deterministic order (by canonical file path), never thread-arrival
  // order — two or more files can independently fail to parse in the
  // same wall-clock window under real concurrency.
  errors.sort_by(|a, b| a.0.cmp(&b.0));
  Err(DriverError::Parse(
    errors.into_iter().flat_map(|(_, e)| e).collect(),
  ))
}

fn parallel_type_check_level(
  graph: &RequireGraph,
  level: &[PathBuf],
  jobs: usize,
  cache: &QueryCache,
  // Plan 49: narrower than plan 48's own `&dyn CacheReporter` — this
  // module shares `reporter` across `std::thread::scope` worker
  // threads, which needs `Sync`. `SilentReporter`/`VerboseReporter`
  // (this crate's own stateless implementors, the only ones a
  // multi-file `--jobs` compile ever constructs) already satisfy this
  // for free; `cache.rs`'s own `CountingReporter` test double (which
  // uses a `RefCell` and is deliberately single-threaded) is
  // untouched, since the plain `CacheReporter` trait itself keeps no
  // `Sync` bound.
  reporter: &(dyn CacheReporter + Sync),
) -> Result<(), DriverError> {
  let queue: Mutex<VecDeque<&PathBuf>> = Mutex::new(level.iter().collect());
  let errors: Mutex<Vec<(PathBuf, Vec<emerald_sema::Diagnostic>)>> = Mutex::new(Vec::new());

  std::thread::scope(|scope| {
    for _ in 0..jobs {
      scope.spawn(|| {
        loop {
          let path = { queue.lock().unwrap().pop_front() };
          let Some(path) = path else { break };
          let items = closure_items(graph, path);
          let program = Program { items };
          let key = cache.key_for_many(&closure_hashes(graph, path));
          let label = path.to_string_lossy().to_string();
          if let Err(diags) = cache.type_check_query(key, &program, &label, reporter) {
            errors.lock().unwrap().push((path.clone(), diags));
          }
        }
      });
    }
  });

  let mut errors = errors.into_inner().unwrap();
  if errors.is_empty() {
    return Ok(());
  }
  errors.sort_by(|a, b| a.0.cmp(&b.0));
  Err(DriverError::Sema(
    errors.into_iter().flat_map(|(_, e)| e).collect(),
  ))
}

fn parallel_codegen_level(
  graph: &RequireGraph,
  level: &[PathBuf],
  jobs: usize,
  cache: &QueryCache,
  // Plan 49: narrower than plan 48's own `&dyn CacheReporter` — this
  // module shares `reporter` across `std::thread::scope` worker
  // threads, which needs `Sync`. `SilentReporter`/`VerboseReporter`
  // (this crate's own stateless implementors, the only ones a
  // multi-file `--jobs` compile ever constructs) already satisfy this
  // for free; `cache.rs`'s own `CountingReporter` test double (which
  // uses a `RefCell` and is deliberately single-threaded) is
  // untouched, since the plain `CacheReporter` trait itself keeps no
  // `Sync` bound.
  reporter: &(dyn CacheReporter + Sync),
  trace: &Mutex<Vec<TraceRecord>>,
) -> Result<Vec<(PathBuf, PathBuf)>, DriverError> {
  let _ = cache; // this leaf's own scoped codegen isn't wired into plan 48's disk cache yet — see the plan's own commit-message disclosure.
  let _ = reporter;
  let queue: Mutex<VecDeque<&PathBuf>> = Mutex::new(level.iter().collect());
  let results: Mutex<Vec<Result<(PathBuf, PathBuf), String>>> = Mutex::new(Vec::new());
  let delay_ms: u64 = std::env::var("EMERALD_TEST_COMPILE_DELAY_MS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(0);
  let trace_enabled = std::env::var("EMERALD_TRACE_COMPILE").as_deref() == Ok("1");
  // Shared zero point every worker's start/end nanos are relative to,
  // so a test comparing two records' [start, end] windows for overlap
  // is comparing genuinely comparable timestamps, not each worker's
  // own independent clock origin.
  let level_start = Instant::now();

  std::thread::scope(|scope| {
    for _ in 0..jobs {
      scope.spawn(|| {
        loop {
          let path = { queue.lock().unwrap().pop_front() };
          let Some(path) = path else { break };
          let start = Instant::now();
          let thread_id = format!("{:?}", std::thread::current().id());

          if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
          }

          let items = closure_items(graph, path);
          let program = Program { items };
          let own_names = own_function_names(graph, path);
          let is_entry = *path == graph.entry;
          let obj_path = std::env::temp_dir().join(format!(
            "emerald_par_{}_{}.o",
            process::id(),
            raw_label(path)
          ));
          let outcome =
            emerald_codegen::compile_to_object_scoped(&program, &own_names, is_entry, &obj_path)
              .map(|()| (path.clone(), obj_path));

          if trace_enabled {
            let end = Instant::now();
            trace.lock().unwrap().push(TraceRecord {
              file: path.clone(),
              thread_id,
              start_nanos: start.duration_since(level_start).as_nanos(),
              end_nanos: end.duration_since(level_start).as_nanos(),
            });
          }

          results.lock().unwrap().push(outcome);
        }
      });
    }
  });

  let results = results.into_inner().unwrap();
  let mut objs = Vec::new();
  let mut errs = Vec::new();
  for r in results {
    match r {
      Ok(pair) => objs.push(pair),
      Err(e) => errs.push(e),
    }
  }
  if !errs.is_empty() {
    return Err(DriverError::Codegen(errs.join("; ")));
  }
  Ok(objs)
}

fn raw_label(path: &Path) -> String {
  path
    .file_stem()
    .map(|s| s.to_string_lossy().to_string())
    .unwrap_or_else(|| "file".to_string())
}

fn dump_trace(trace: &Mutex<Vec<TraceRecord>>) {
  let Ok(out_path) = std::env::var("EMERALD_TRACE_COMPILE_OUT") else {
    return;
  };
  let records = trace.lock().unwrap();
  let text = records
    .iter()
    .map(|r| {
      format!(
        "{}\t{}\t{}\t{}",
        r.file.display(),
        r.thread_id,
        r.start_nanos,
        r.end_nanos
      )
    })
    .collect::<Vec<_>>()
    .join("\n");
  std::fs::write(out_path, text).ok();
}

/// `link`'s own multi-object sibling (plan 49's Decision log: linking
/// gains N object-file arguments, stays otherwise unchanged and stays
/// single-threaded) — every level's own emitted `.o` gets passed to
/// `cc`, ahead of the embedded runtime archive, exactly as `link`
/// already does for the single-object case.
fn link_many(obj_paths: Vec<PathBuf>, output_path: PathBuf) -> Result<(), DriverError> {
  let runtime_archive_path =
    std::env::temp_dir().join(format!("libemerald_runtime_par_{}.a", process::id()));
  if let Err(e) = std::fs::write(&runtime_archive_path, RUNTIME_ARCHIVE) {
    for obj in &obj_paths {
      std::fs::remove_file(obj).ok();
    }
    return Err(DriverError::Link(format!(
      "failed to extract the embedded runtime archive: {e}"
    )));
  }
  // Bugfix (benchmark session): matches `build_link_args`'s own
  // `-Wl,--gc-sections` (`emerald-driver/src/lib.rs`) — this is the
  // `--jobs` build's separate `cc` invocation, so it needs the same flag
  // or multi-file builds would keep linking the whole runtime archive
  // in regardless of build.rs's per-function/per-data sections.
  let link_result = Command::new("cc")
    .arg("-no-pie")
    .arg("-Wl,--gc-sections")
    .args(&obj_paths)
    .arg(&runtime_archive_path)
    .arg("-o")
    .arg(&output_path)
    .status();
  for obj in &obj_paths {
    std::fs::remove_file(obj).ok();
  }
  std::fs::remove_file(&runtime_archive_path).ok();
  match link_result {
    Ok(status) if status.success() => Ok(()),
    Ok(_) => Err(DriverError::Link("linking failed".to_string())),
    Err(e) => Err(DriverError::Link(format!("failed to invoke cc: {e}"))),
  }
}
