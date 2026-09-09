//! `emerald repl` / bare `emerald` (plan 47's `leaf-repl`) —
//! subprocess-per-line incremental session over `emerald_driver::
//! compile`. No LLVM JIT: each line is a real, fresh AOT compile +
//! link + run of `prelude + this line`, exactly the same pipeline
//! `emerald <file>` already uses (see this leaf's own Decision log for
//! the real, disclosed per-line latency cost that trades for).

use emerald_driver::cache::{CacheReporter, QueryCache, SilentReporter, VerboseReporter};
use emerald_parser::{Expr, Item, Spanned, Stmt};
use std::io::{self, BufRead, Write};
use std::process::{self, Command};

/// Plan 48's `leaf-verbose-flag-and-consumer-wiring`: `--history-cache
/// <dir>` points the session's `QueryCache` at a stable, reusable
/// directory instead of a throwaway per-process-group temp one — the
/// mechanism the plan's own Decision log's transcript-replay bullet
/// depends on (replaying an *unmodified* transcript twice against the
/// same `--history-cache` directory hits every previously-seen line's
/// `check`/`codegen` cache). Passing `--history-cache` also enables
/// `[cache] ... HIT|MISS` reporting to stderr; without it, the session
/// still caches (so re-submitting the exact same line against the
/// exact same prelude is a real, if narrow, hit — see the Decision
/// log), just silently, against an ephemeral temp directory freed with
/// the session.
fn history_cache_dir(args: &[String]) -> Option<std::path::PathBuf> {
  args
    .iter()
    .position(|a| a == "--history-cache")
    .and_then(|i| args.get(i + 1))
    .map(std::path::PathBuf::from)
}

pub fn run(args: &[String]) {
  println!("Emerald REPL — each line is compiled and run fresh against the session so far.");
  let mut prelude = String::new();

  let history_dir = history_cache_dir(args);
  let verbose = history_dir.is_some();
  let cache_root = history_dir
    .unwrap_or_else(|| std::env::temp_dir().join(format!("emerald-repl-cache-{}", process::id())));
  let cache = QueryCache::new(cache_root.clone());
  let reporter: Box<dyn CacheReporter> = if verbose {
    Box::new(VerboseReporter)
  } else {
    Box::new(SilentReporter)
  };

  let stdin = io::stdin();
  let mut lines = stdin.lock().lines();

  loop {
    print!("emerald> ");
    io::stdout().flush().ok();
    let Some(Ok(line)) = lines.next() else {
      break; // EOF (Ctrl-D) or a real stdin read error — either way, stop.
    };
    let line = line.trim();
    if line == "exit" || line == "quit" {
      break;
    }
    if line.is_empty() {
      continue;
    }
    handle_line(&mut prelude, line, &cache, reporter.as_ref());
  }

  // The ephemeral default (no `--history-cache`) is a real per-process
  // temp directory (`QueryCache::codegen_query` persists real `.o`
  // bytes to disk, not just an in-memory map) — freed with the
  // session, matching the REPL's own subprocess-per-line model.
  if !verbose {
    std::fs::remove_dir_all(&cache_root).ok();
  }
}

/// Classifies `line` (parsed alone, purely to inspect its shape — the
/// candidate actually compiled below is always `prelude + ...`), then
/// either commits it to `prelude` (a declaration, on success only) or
/// compiles-and-runs it once without ever touching `prelude` (a
/// transient expression — auto-printed via a literal `puts(...)`
/// source-text wrap, unless it's already a bare `puts` call).
fn handle_line(prelude: &mut String, line: &str, cache: &QueryCache, reporter: &dyn CacheReporter) {
  let program = match emerald_parser::parse(line) {
    Ok(p) => p,
    Err(errs) => {
      for e in errs {
        eprintln!("{:?}", miette::Report::new(e));
      }
      return;
    }
  };

  // Plan 47's Decision log: a declaration is `Item::Function`/`Class`/
  // `Module`, or a top-level `Item::Stmt` whose `Stmt` is `Let`/
  // `Assign`/`MultiAssign`/`SetField`/`SetIndex`; a transient
  // expression is `Item::Stmt(Stmt::Expr(_))`, the only remaining
  // shape a one-line REPL prompt is expected to see. Anything else
  // (an `if`/`while`/`begin`/... spanning just one physical line) is
  // treated as a declaration too — appended verbatim, never
  // `puts`-wrapped — since it isn't `Stmt::Expr` either.
  let is_transient = matches!(
    program.items.as_slice(),
    [Item::Stmt(Spanned {
      node: Stmt::Expr(_),
      ..
    })]
  );

  if !is_transient {
    let candidate = format!("{prelude}{line}\n");
    if let Ok(stdout) = compile_and_run(&candidate, cache, reporter) {
      print!("{stdout}");
      io::stdout().flush().ok();
      prelude.push_str(line);
      prelude.push('\n');
    }
    return;
  }

  let already_puts = matches!(
    program.items.as_slice(),
    [Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(name, _),
        ..
      }),
      ..
    })] if name == "puts"
  );
  // `puts` is a Stmt-level keyword taking one bare `Expr` (`"puts"
  // <arg:Expr>`) — this grammar has no parenthesized-expression-
  // grouping syntax at all, so `puts(<line>)` would be a real parse
  // error here, not a call. `puts <line>` works because `Expr`
  // greedily consumes the line's whole precedence chain (e.g. `x + 5`
  // parses as one `Expr`, not `x` followed by a stray `+ 5`).
  let wrapped_line = if already_puts {
    line.to_string()
  } else {
    format!("puts {line}")
  };
  let candidate = format!("{prelude}{wrapped_line}\n");
  if let Ok(stdout) = compile_and_run(&candidate, cache, reporter) {
    print!("{stdout}");
    io::stdout().flush().ok();
  }
  // A transient expression's text is never appended to `prelude`.
}

/// Compiles+links+runs `candidate` via `emerald_driver::compile_cached`
/// (plan 48) instead of the plain `compile` — every REPL line goes
/// through the session's own `QueryCache` (an ephemeral per-session
/// temp directory by default, or `--history-cache <dir>`'s stable one
/// — see this module's own Decision-log comment on `run`). `Err(())`
/// means a diagnostic was already printed to stderr — the caller has
/// nothing further to do.
fn compile_and_run(
  candidate: &str,
  cache: &QueryCache,
  reporter: &dyn CacheReporter,
) -> Result<String, ()> {
  let out_path = std::env::temp_dir().join(format!("emerald_repl_{}.out", process::id()));
  let result = emerald_driver::compile_cached(candidate, "<repl>", &out_path, cache, reporter);
  match result {
    Ok(()) => {
      let output = Command::new(&out_path).output();
      std::fs::remove_file(&out_path).ok();
      match output {
        Ok(o) => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Err(e) => {
          eprintln!("error: failed to run compiled line: {e}");
          Err(())
        }
      }
    }
    Err(e) => {
      crate::report_driver_error(e, Some(("<repl>", candidate)));
      Err(())
    }
  }
}
