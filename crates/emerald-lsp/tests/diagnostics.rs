//! Real end-to-end proof of plan 17's `leaf-lsp-server` acceptance
//! criteria — drives `emerald_lsp::run` over `Connection::memory()`
//! exactly as a real client speaks LSP over stdio, never calling
//! `check_diagnostics`/parsing helpers directly.

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::notification::{
  DidOpenTextDocument, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Initialize, Request as _};
use lsp_types::{
  DidOpenTextDocumentParams, InitializeParams, InitializeResult, InitializedParams,
  PublishDiagnosticsParams, TextDocumentItem, TextDocumentSyncCapability, TextDocumentSyncKind,
};

/// Spawns the server against one end of an in-memory connection,
/// returns the other end (the "client" side the test drives) plus the
/// join handle.
fn start_server() -> (Connection, std::thread::JoinHandle<()>) {
  let (server_conn, client_conn) = Connection::memory();
  let handle = std::thread::spawn(move || {
    emerald_lsp::run(&server_conn).expect("server loop should not error");
  });
  (client_conn, handle)
}

fn initialize(client: &Connection) -> InitializeResult {
  client
    .sender
    .send(Message::Request(Request::new(
      RequestId::from(1),
      Initialize::METHOD.to_string(),
      InitializeParams::default(),
    )))
    .unwrap();

  let response = match client.receiver.recv().unwrap() {
    Message::Response(resp) => resp,
    other => panic!("expected a Response to initialize, got {other:?}"),
  };
  let result: InitializeResult = serde_json::from_value(response.response_result.unwrap()).unwrap();

  client
    .sender
    .send(Message::Notification(Notification::new(
      Initialized::METHOD.to_string(),
      InitializedParams {},
    )))
    .unwrap();

  result
}

fn did_open(client: &Connection, uri: &str, text: &str) {
  let params = DidOpenTextDocumentParams {
    text_document: TextDocumentItem::new(
      uri.parse().unwrap(),
      "emerald".to_string(),
      1,
      text.to_string(),
    ),
  };
  client
    .sender
    .send(Message::Notification(Notification::new(
      DidOpenTextDocument::METHOD.to_string(),
      params,
    )))
    .unwrap();
}

fn recv_diagnostics(client: &Connection) -> PublishDiagnosticsParams {
  match client
    .receiver
    .recv_timeout(std::time::Duration::from_secs(5))
    .expect("expected a publishDiagnostics notification")
  {
    Message::Notification(not) => {
      assert_eq!(not.method, PublishDiagnostics::METHOD);
      serde_json::from_value(not.params).unwrap()
    }
    other => panic!("expected a Notification, got {other:?}"),
  }
}

#[test]
fn initialize_advertises_full_text_document_sync() {
  let (client, handle) = start_server();
  let result = initialize(&client);
  assert_eq!(
    result.capabilities.text_document_sync,
    Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL))
  );
  drop(client);
  handle.join().unwrap();
}

#[test]
fn a_parse_error_publishes_one_diagnostic_at_the_real_utf16_column() {
  let (client, handle) = start_server();
  initialize(&client);

  // Plan 13's own worked parse-error example — verified this session
  // (via a real `emerald` CLI invocation) to still fail to parse at
  // byte offset 11 (the `+`), line 1.
  did_open(&client, "file:///bad.em", "x: Int64 = +\n");
  let published = recv_diagnostics(&client);

  assert_eq!(published.diagnostics.len(), 1);
  let diag = &published.diagnostics[0];
  assert_eq!(diag.range.start.line, 0);
  assert_eq!(diag.range.start.character, 11);

  drop(client);
  handle.join().unwrap();
}

#[test]
fn a_sema_type_mismatch_publishes_a_diagnostic_at_the_real_b_position() {
  // Plan 22's `leaf-span-rendering` AC2: `emerald_sema::Diagnostic` now
  // carries a real span, so the LSP no longer anchors sema diagnostics
  // at the whole document — it should point at the same `b` on line 2
  // that the CLI's miette rendering already proves (see
  // `emerald-cli/tests/hello_em.rs`'s
  // `sema_type_mismatch_renders_a_miette_source_snippet_with_a_caret_at_b`).
  let (client, handle) = start_server();
  initialize(&client);

  did_open(
    &client,
    "file:///bad_types.em",
    "def add(a: Int64, b: String) -> Int64\n  a + b\nend\n",
  );
  let published = recv_diagnostics(&client);

  assert_eq!(published.diagnostics.len(), 1);
  let diag = &published.diagnostics[0];
  assert_eq!(diag.range.start.line, 1);
  assert_eq!(diag.range.start.character, 6);
  assert!(diag.range.end.character > diag.range.start.character);
  assert!(
    diag.message.contains("Int64") && diag.message.contains("String"),
    "message should name both types: {}",
    diag.message
  );

  drop(client);
  handle.join().unwrap();
}

#[test]
fn a_real_working_program_publishes_no_diagnostics() {
  let (client, handle) = start_server();
  initialize(&client);

  let hello_src = include_str!("../../../examples/hello.em");
  did_open(&client, "file:///hello.em", hello_src);
  let published = recv_diagnostics(&client);

  assert!(published.diagnostics.is_empty());

  drop(client);
  handle.join().unwrap();
}
