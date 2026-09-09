//! `emerald repl` / bare `emerald` (plan 47's `leaf-repl`) —
//! subprocess-per-line incremental session over `emerald_driver::
//! compile`. No LLVM JIT: each line is a real, fresh AOT compile +
//! link + run of `prelude + this line`, exactly the same pipeline
//! `emerald <file>` already uses (see this leaf's own Decision log for
//! the real, disclosed per-line latency cost that trades for).

use emerald_parser::{Expr, Item, Stmt};
use std::io::{self, BufRead, Write};
use std::process::{self, Command};

pub fn run() {
  println!("Emerald REPL — each line is compiled and run fresh against the session so far.");
  let mut prelude = String::new();
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
    handle_line(&mut prelude, line);
  }
}

/// Classifies `line` (parsed alone, purely to inspect its shape — the
/// candidate actually compiled below is always `prelude + ...`), then
/// either commits it to `prelude` (a declaration, on success only) or
/// compiles-and-runs it once without ever touching `prelude` (a
/// transient expression — auto-printed via a literal `puts(...)`
/// source-text wrap, unless it's already a bare `puts` call).
fn handle_line(prelude: &mut String, line: &str) {
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
  let is_transient = matches!(program.items.as_slice(), [Item::Stmt(Stmt::Expr(_))]);

  if !is_transient {
    let candidate = format!("{prelude}{line}\n");
    if let Ok(stdout) = compile_and_run(&candidate) {
      print!("{stdout}");
      io::stdout().flush().ok();
      prelude.push_str(line);
      prelude.push('\n');
    }
    return;
  }

  let already_puts = matches!(
    program.items.as_slice(),
    [Item::Stmt(Stmt::Expr(Expr::Call(name, _)))] if name == "puts"
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
  if let Ok(stdout) = compile_and_run(&candidate) {
    print!("{stdout}");
    io::stdout().flush().ok();
  }
  // A transient expression's text is never appended to `prelude`.
}

/// Compiles+links+runs `candidate` via the ordinary `emerald_driver::
/// compile` pipeline. `Err(())` means a diagnostic was already printed
/// to stderr — the caller has nothing further to do.
fn compile_and_run(candidate: &str) -> Result<String, ()> {
  let out_path = std::env::temp_dir().join(format!("emerald_repl_{}.out", process::id()));
  let result = emerald_driver::compile(candidate, "<repl>", &out_path);
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
      crate::report_driver_error(e);
      Err(())
    }
  }
}
