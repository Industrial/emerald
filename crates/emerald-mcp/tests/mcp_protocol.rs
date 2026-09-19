//! Real end-to-end proof of plan 17's `leaf-mcp-server` acceptance
//! criteria — spawns the actual compiled `emerald-mcp` binary as a
//! child process and speaks real MCP-over-stdio to it via `rmcp`'s own
//! client transport, never calling this crate's tool handlers
//! in-process.

use rmcp::model::{object, CallToolRequestParams};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;

async fn connect() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
  let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_emerald-mcp"));
  let transport = TokioChildProcess::new(cmd).expect("spawn emerald-mcp");
  ().serve(transport).await.expect("MCP initialize handshake")
}

async fn call(
  client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
  tool: &'static str,
  arguments: serde_json::Value,
) -> serde_json::Value {
  let params = CallToolRequestParams::new(tool).with_arguments(object(arguments));
  let response = client
    .call_tool_once(params)
    .await
    .unwrap_or_else(|e| panic!("calling `{tool}` failed: {e}"));
  let rmcp::model::CallToolResponse::Complete(result) = response else {
    panic!("`{tool}` returned a non-complete response");
  };
  assert_ne!(
    result.is_error,
    Some(true),
    "`{tool}` reported is_error=true: {result:?}"
  );
  result
    .into_typed::<serde_json::Value>()
    .unwrap_or_else(|e| panic!("`{tool}`'s response did not deserialize: {e}"))
}

#[tokio::test]
async fn initialize_and_tools_list_advertises_all_three_tools() {
  let client = connect().await;
  let tools = client.list_all_tools().await.unwrap();
  let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
  assert!(names.contains(&"check_source".to_string()), "{names:?}");
  assert!(names.contains(&"compile_and_run".to_string()), "{names:?}");
  assert!(names.contains(&"list_examples".to_string()), "{names:?}");
  client.cancel().await.unwrap();
}

#[tokio::test]
async fn check_source_reports_a_real_parse_error_and_a_clean_pass() {
  let client = connect().await;

  // Plan 13's own worked parse-error example — verified this session
  // (via a real `emerald` CLI invocation) to still fail to parse.
  let bad = call(
    &client,
    "check_source",
    serde_json::json!({ "source": "x: Int64 = +\n" }),
  )
  .await;
  let diagnostics = bad["diagnostics"].as_array().expect("diagnostics array");
  assert_eq!(diagnostics.len(), 1, "{bad}");
  assert!(diagnostics[0]["message"].as_str().is_some(), "{bad}");
  assert!(diagnostics[0]["line"].as_u64().is_some(), "{bad}");
  assert!(diagnostics[0]["column"].as_u64().is_some(), "{bad}");

  let hello_src = include_str!("../../../examples/hello.em");
  let good = call(
    &client,
    "check_source",
    serde_json::json!({ "source": hello_src }),
  )
  .await;
  assert_eq!(good["diagnostics"].as_array().unwrap().len(), 0, "{good}");

  client.cancel().await.unwrap();
}

#[tokio::test]
async fn check_source_reports_a_real_line_and_column_for_a_sema_type_mismatch() {
  // Plan 22's `leaf-span-rendering` AC3: `emerald_sema::Diagnostic` now
  // carries a real span, so `check_source` no longer reports `line`/
  // `column` as absent for sema diagnostics — same worked example as
  // the CLI (`sema_type_mismatch_renders_a_miette_source_snippet_with_a_caret_at_b`)
  // and the LSP (`a_sema_type_mismatch_publishes_a_diagnostic_at_the_real_b_position`)
  // tests, pointing at the same `b` on line 2.
  let client = connect().await;

  let result = call(
    &client,
    "check_source",
    serde_json::json!({ "source": "fn add(a: Int64, b: String): Int64 do\n  a + b\nend\n" }),
  )
  .await;
  let diagnostics = result["diagnostics"].as_array().expect("diagnostics array");
  assert_eq!(diagnostics.len(), 1, "{result}");
  assert_eq!(
    diagnostics[0]["kind"],
    serde_json::json!("sema"),
    "{result}"
  );
  assert_eq!(diagnostics[0]["line"], serde_json::json!(2), "{result}");
  assert_eq!(diagnostics[0]["column"], serde_json::json!(7), "{result}");
  let message = diagnostics[0]["message"].as_str().expect("message string");
  assert!(
    message.contains("Int64") && message.contains("String"),
    "message should name both types: {message}"
  );

  client.cancel().await.unwrap();
}

#[tokio::test]
async fn compile_and_run_actually_compiles_links_and_runs_hello_em() {
  let client = connect().await;

  let hello_src = include_str!("../../../examples/hello.em");
  let result = call(
    &client,
    "compile_and_run",
    serde_json::json!({ "source": hello_src }),
  )
  .await;

  assert_eq!(result["compiled"], serde_json::json!(true), "{result}");
  assert_eq!(result["exit_code"], serde_json::json!(0), "{result}");
  assert_eq!(result["stdout"], serde_json::json!("42\n"), "{result}");

  client.cancel().await.unwrap();
}
