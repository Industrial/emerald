//! MCP (Model Context Protocol) server for Emerald (plan 17's
//! `leaf-mcp-server`) — gives an AI coding agent structured tool access
//! to `emerald-driver`'s check/compile pipeline instead of shelling out
//! to `emerald-cli` and scraping stderr text. stdio transport, three
//! tools: `check_source`, `compile_and_run`, `list_examples`.
//!
//! This is the one deliberate exception to the workspace's usual
//! no-async posture (see plan 17's Decision log): `rmcp`'s stdio
//! transport is tokio-based, and this is a brand-new binary crate, not
//! a change to `emerald-cli`'s existing synchronous pipeline.

use emerald_driver::DriverError;
use rmcp::{
  handler::server::{
    router::tool::ToolRouter,
    wrapper::{Json, Parameters},
  },
  model::{ServerCapabilities, ServerInfo},
  tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Converts a byte offset into `source` to a 1-based (line, column) —
/// byte columns, not UTF-16, since this is a plain-text tool result,
/// not an LSP wire message.
fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
  let offset = offset.min(source.len());
  let mut line = 1usize;
  let mut last_newline: Option<usize> = None;
  for (i, b) in source.as_bytes()[..offset].iter().enumerate() {
    if *b == b'\n' {
      line += 1;
      last_newline = Some(i);
    }
  }
  let column = match last_newline {
    Some(nl) => offset - nl,
    None => offset + 1,
  };
  (line, column)
}

#[derive(Debug, Serialize, JsonSchema)]
struct CheckDiagnostic {
  /// "parse" or "sema".
  kind: &'static str,
  message: String,
  /// 1-based line; `None` only for "internal" diagnostics (codegen/link
  /// failures, which carry no source position at all).
  line: Option<usize>,
  /// 1-based byte column; `None` only for "internal" diagnostics.
  column: Option<usize>,
}

/// Builds a parse diagnostic from anything implementing `miette::Diagnostic`
/// (the trait `emerald_parser::ParseError` implements) — generic over `E`
/// so this crate never needs to name `emerald_parser::ParseError` directly.
fn parse_diagnostic<E: miette::Diagnostic + std::fmt::Display>(
  e: &E,
  source: &str,
) -> CheckDiagnostic {
  let message = e.to_string();
  let (line, column) = e
    .labels()
    .and_then(|mut labels| labels.next())
    .map(|label| offset_to_line_col(source, label.offset()))
    .unwrap_or((1, 1));
  CheckDiagnostic {
    kind: "parse",
    message,
    line: Some(line),
    column: Some(column),
  }
}

/// Takes the message and span by value rather than naming
/// `emerald_sema::Diagnostic` directly — this crate doesn't otherwise
/// need `emerald-sema` as a dependency, since `emerald_driver::DriverError`
/// already re-exports it structurally.
fn sema_diagnostic(source: &str, message: String, span: (usize, usize)) -> CheckDiagnostic {
  let (line, column) = offset_to_line_col(source, span.0);
  CheckDiagnostic {
    kind: "sema",
    message,
    line: Some(line),
    column: Some(column),
  }
}

fn internal_diagnostic(message: String) -> CheckDiagnostic {
  CheckDiagnostic {
    kind: "internal",
    message,
    line: None,
    column: None,
  }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CheckSourceRequest {
  /// Emerald source text to parse and type-check in memory.
  source: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CheckSourceResponse {
  /// Empty when `source` compiles cleanly.
  diagnostics: Vec<CheckDiagnostic>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CompileAndRunRequest {
  /// Emerald source text to compile, link, and run.
  source: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CompileAndRunResponse {
  /// `false` means compilation failed — see `diagnostics` — and
  /// `exit_code`/`stdout`/`stderr` are absent.
  compiled: bool,
  exit_code: Option<i32>,
  stdout: Option<String>,
  stderr: Option<String>,
  diagnostics: Vec<CheckDiagnostic>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ExampleFile {
  name: String,
  contents: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListExamplesResponse {
  /// `examples/README.md`'s contents, describing every file below.
  readme: String,
  examples: Vec<ExampleFile>,
}

/// The checked-in `examples/*.em` corpus, embedded at compile time so
/// `list_examples` works regardless of the server's runtime `cwd` —
/// `examples/packages/` is a separate multi-package example and
/// deliberately excluded (see plan 17's leaf spec).
static EXAMPLE_SOURCES: &[(&str, &str)] = &[
  (
    "classes.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/classes.em"
    )),
  ),
  (
    "closures.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/closures.em"
    )),
  ),
  (
    "collections.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/collections.em"
    )),
  ),
  (
    "control_flow.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/control_flow.em"
    )),
  ),
  (
    "exceptions.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/exceptions.em"
    )),
  ),
  (
    "hello.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/hello.em"
    )),
  ),
  (
    "interfaces_generics.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/interfaces_generics.em"
    )),
  ),
  (
    "modules.em",
    include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../examples/modules.em"
    )),
  ),
];

static EXAMPLES_README: &str = include_str!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/../../examples/README.md"
));

/// A process-wide counter so concurrent `compile_and_run` calls (same
/// `process::id()`) don't collide on the same scratch file names.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct EmeraldServer {
  tool_router: ToolRouter<Self>,
}

impl EmeraldServer {
  fn new() -> Self {
    Self {
      tool_router: Self::tool_router(),
    }
  }
}

#[tool_router]
impl EmeraldServer {
  #[tool(
    description = "Parse and type-check Emerald source text in memory. Returns structured diagnostics with a real 1-based line/column for both parse and sema errors. Empty diagnostics means the source compiles cleanly."
  )]
  async fn check_source(
    &self,
    Parameters(req): Parameters<CheckSourceRequest>,
  ) -> Json<CheckSourceResponse> {
    let diagnostics = match emerald_driver::check(&req.source, "mcp_check_source.em") {
      Ok(()) => vec![],
      Err(DriverError::Parse(errs)) => errs
        .iter()
        .map(|e| parse_diagnostic(e, &req.source))
        .collect(),
      Err(DriverError::Sema(diags)) => diags
        .iter()
        .map(|d| sema_diagnostic(&req.source, d.message.clone(), d.span))
        .collect(),
      // `check()` only ever parses + type-checks — codegen/link are
      // unreachable here, but the match must stay exhaustive.
      Err(DriverError::Codegen(e)) => vec![internal_diagnostic(e)],
      Err(DriverError::Link(e)) => vec![internal_diagnostic(e)],
      // Plan 49: only ever produced by `emerald_driver::parallel::
      // compile_parallel`, which `check()` never calls.
      Err(DriverError::Require(e)) => vec![internal_diagnostic(e)],
    };
    Json(CheckSourceResponse { diagnostics })
  }

  #[tool(
    description = "Write Emerald source text to a scratch file, compile and link it via emerald-driver, then actually run the resulting binary and return its captured stdout, stderr, and exit code."
  )]
  async fn compile_and_run(
    &self,
    Parameters(req): Parameters<CompileAndRunRequest>,
  ) -> Json<CompileAndRunResponse> {
    let nonce = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let src_path = dir.join(format!("emerald_mcp_{}_{nonce}.em", std::process::id()));
    let out_path = dir.join(format!("emerald_mcp_{}_{nonce}_out", std::process::id()));

    if let Err(e) = std::fs::write(&src_path, &req.source) {
      return Json(CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: vec![internal_diagnostic(format!(
          "failed to write scratch source file: {e}"
        ))],
      });
    }

    let name = src_path.to_string_lossy().into_owned();
    let response = match emerald_driver::compile(&req.source, &name, &out_path) {
      Ok(()) => match std::process::Command::new(&out_path).output() {
        Ok(output) => CompileAndRunResponse {
          compiled: true,
          exit_code: output.status.code(),
          stdout: Some(String::from_utf8_lossy(&output.stdout).into_owned()),
          stderr: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
          diagnostics: vec![],
        },
        Err(e) => CompileAndRunResponse {
          compiled: false,
          exit_code: None,
          stdout: None,
          stderr: None,
          diagnostics: vec![internal_diagnostic(format!(
            "failed to run compiled binary: {e}"
          ))],
        },
      },
      Err(DriverError::Parse(errs)) => CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: errs
          .iter()
          .map(|e| parse_diagnostic(e, &req.source))
          .collect(),
      },
      Err(DriverError::Sema(diags)) => CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: diags
          .iter()
          .map(|d| sema_diagnostic(&req.source, d.message.clone(), d.span))
          .collect(),
      },
      Err(DriverError::Codegen(e)) => CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: vec![internal_diagnostic(format!("codegen error: {e}"))],
      },
      Err(DriverError::Link(e)) => CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: vec![internal_diagnostic(format!("link error: {e}"))],
      },
      // Plan 49: only ever produced by `emerald_driver::parallel::
      // compile_parallel`, which `compile()` never calls.
      Err(DriverError::Require(e)) => CompileAndRunResponse {
        compiled: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        diagnostics: vec![internal_diagnostic(format!("require error: {e}"))],
      },
    };

    std::fs::remove_file(&src_path).ok();
    std::fs::remove_file(&out_path).ok();
    Json(response)
  }

  #[tool(
    description = "Return the checked-in examples/*.em corpus (file names + contents) plus examples/README.md's descriptions, so an agent can ground itself in real, working Emerald syntax instead of guessing at unimplemented Ruby features."
  )]
  async fn list_examples(&self) -> Json<ListExamplesResponse> {
    Json(ListExamplesResponse {
      readme: EXAMPLES_README.to_string(),
      examples: EXAMPLE_SOURCES
        .iter()
        .map(|(name, contents)| ExampleFile {
          name: (*name).to_string(),
          contents: (*contents).to_string(),
        })
        .collect(),
    })
  }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EmeraldServer {
  fn get_info(&self) -> ServerInfo {
    ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
      "Emerald compiler tools: check_source (parse+typecheck in memory), \
         compile_and_run (compile, link, and actually run source), and \
         list_examples (the real, working examples/*.em corpus).",
    )
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let server = EmeraldServer::new().serve(rmcp::transport::stdio()).await?;
  server.waiting().await?;
  Ok(())
}
