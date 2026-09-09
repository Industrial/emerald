//! Real end-to-end proof of plan 21's `leaf-completion` acceptance
//! criteria — drives `emerald_lsp::run` over `Connection::memory()`,
//! never calling its internal helpers directly.

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::notification::{DidOpenTextDocument, Initialized, Notification as _};
use lsp_types::request::{Completion, Initialize, Request as _};
use lsp_types::{
  CompletionContext, CompletionParams, CompletionResponse, CompletionTriggerKind,
  DidOpenTextDocumentParams, InitializeParams, InitializedParams, PartialResultParams, Position,
  TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, WorkDoneProgressParams,
};

fn start_server() -> (Connection, std::thread::JoinHandle<()>) {
  let (server_conn, client_conn) = Connection::memory();
  let handle = std::thread::spawn(move || {
    emerald_lsp::run(&server_conn).expect("server loop should not error");
  });
  (client_conn, handle)
}

fn initialize(client: &Connection) {
  client
    .sender
    .send(Message::Request(Request::new(
      RequestId::from(1),
      Initialize::METHOD.to_string(),
      InitializeParams::default(),
    )))
    .unwrap();
  client.receiver.recv().unwrap();
  client
    .sender
    .send(Message::Notification(Notification::new(
      Initialized::METHOD.to_string(),
      InitializedParams {},
    )))
    .unwrap();
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
  client.receiver.recv().unwrap(); // diagnostics from the didOpen
}

fn complete(client: &Connection, uri: &str, line: u32, character: u32) -> CompletionResponse {
  let params = CompletionParams {
    text_document_position: TextDocumentPositionParams {
      text_document: TextDocumentIdentifier {
        uri: uri.parse().unwrap(),
      },
      position: Position::new(line, character),
    },
    work_done_progress_params: WorkDoneProgressParams::default(),
    partial_result_params: PartialResultParams::default(),
    context: Some(CompletionContext {
      trigger_kind: CompletionTriggerKind::INVOKED,
      trigger_character: None,
    }),
  };
  client
    .sender
    .send(Message::Request(Request::new(
      RequestId::from(2),
      Completion::METHOD.to_string(),
      params,
    )))
    .unwrap();
  match client.receiver.recv().unwrap() {
    Message::Response(resp) => serde_json::from_value(resp.response_result.unwrap()).unwrap(),
    other => panic!("expected a Response, got {other:?}"),
  }
}

fn labels(response: &CompletionResponse) -> Vec<String> {
  match response {
    CompletionResponse::Array(items) => items.iter().map(|i| i.label.clone()).collect(),
    CompletionResponse::List(list) => list.items.iter().map(|i| i.label.clone()).collect(),
  }
}

const HELLO_SRC: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

#[test]
fn completion_over_hello_em_includes_add_and_core_keywords() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///hello.em", HELLO_SRC);

  let response = complete(&client, "file:///hello.em", 4, 5);
  let names = labels(&response);

  assert!(
    names.contains(&"add".to_string()),
    "missing 'add': {names:?}"
  );
  for kw in ["def", "end", "if", "class"] {
    assert!(
      names.contains(&kw.to_string()),
      "missing keyword '{kw}': {names:?}"
    );
  }

  drop(client);
  handle.join().unwrap();
}

#[test]
fn completion_over_an_unparseable_buffer_still_returns_the_keyword_list() {
  let (client, handle) = start_server();
  initialize(&client);
  // Plan 13's known-bad buffer: a dangling binary operator.
  did_open(&client, "file:///bad.em", "x: Int64 = +\n");

  let response = complete(&client, "file:///bad.em", 0, 12);
  let names = labels(&response);

  assert!(!names.is_empty(), "expected a non-empty keyword-only list");
  for kw in ["def", "end", "if", "class"] {
    assert!(
      names.contains(&kw.to_string()),
      "missing keyword '{kw}': {names:?}"
    );
  }
  // A parse failure must never surface a function/class name that
  // doesn't really exist in this buffer.
  assert!(!names.contains(&"add".to_string()));

  drop(client);
  handle.join().unwrap();
}
