//! Real end-to-end proof of plan 21's `leaf-semantic-tokens`
//! acceptance criteria — drives `emerald_lsp::run` over
//! `Connection::memory()`, never calling its internal helpers
//! directly, and decodes the wire's delta-encoded `data` array back
//! into absolute `(line, character, length, token_type)` positions
//! checked against real offsets found in the source text itself.

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::notification::{DidOpenTextDocument, Initialized, Notification as _};
use lsp_types::request::{Initialize, Request as _, SemanticTokensFullRequest};
use lsp_types::{
  InitializeParams, InitializedParams, PartialResultParams, SemanticTokensParams,
  SemanticTokensResult, TextDocumentIdentifier, TextDocumentItem, WorkDoneProgressParams,
};

const TOKEN_KEYWORD: u32 = 0;
const TOKEN_CLASS: u32 = 2;
const TOKEN_FUNCTION: u32 = 3;

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
  let params = lsp_types::DidOpenTextDocumentParams {
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

/// Absolute `(line, character, length, token_type)` triples, decoded
/// from the wire's delta-encoded token stream (each token's line and
/// character are relative to the *previous* token, per the LSP spec).
fn semantic_tokens(client: &Connection, uri: &str) -> Vec<(u32, u32, u32, u32)> {
  let params = SemanticTokensParams {
    text_document: TextDocumentIdentifier {
      uri: uri.parse().unwrap(),
    },
    work_done_progress_params: WorkDoneProgressParams::default(),
    partial_result_params: PartialResultParams::default(),
  };
  client
    .sender
    .send(Message::Request(Request::new(
      RequestId::from(2),
      SemanticTokensFullRequest::METHOD.to_string(),
      params,
    )))
    .unwrap();
  let result: Option<SemanticTokensResult> = match client.receiver.recv().unwrap() {
    Message::Response(resp) => serde_json::from_value(resp.response_result.unwrap()).unwrap(),
    other => panic!("expected a Response, got {other:?}"),
  };
  let Some(SemanticTokensResult::Tokens(tokens)) = result else {
    panic!("expected a Tokens result, got {result:?}");
  };

  let mut line = 0u32;
  let mut character = 0u32;
  let mut out = Vec::with_capacity(tokens.data.len());
  for tok in tokens.data {
    if tok.delta_line == 0 {
      character += tok.delta_start;
    } else {
      line += tok.delta_line;
      character = tok.delta_start;
    }
    out.push((line, character, tok.length, tok.token_type));
  }
  out
}

const HELLO_SRC: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
const CLASSES_SRC: &str = "class Counter\n  value: Int64\n\n  def initialize(start: Int64) -> Void\n    @value = start\n  end\n\n  def value -> Int64\n    @value\n  end\n\n  def add(n: Int64) -> Int64\n    @value + n\n  end\nend\n\nc: Counter = Counter.new(10)\nputs c.value\nputs c.add(5)\n\nclass Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def sum -> Float64\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

#[test]
fn hello_em_tags_add_as_function_and_def_end_puts_as_keywords() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///hello.em", HELLO_SRC);

  let tokens = semantic_tokens(&client, "file:///hello.em");

  // "puts add(20, 22)" is line 4; "add" starts at character 5.
  assert!(HELLO_SRC.lines().nth(4) == Some("puts add(20, 22)"));
  assert!(
    tokens.contains(&(4, 5, 3, TOKEN_FUNCTION)),
    "expected add-as-function at (4,5,3): {tokens:?}"
  );
  assert!(
    tokens.contains(&(4, 0, 4, TOKEN_KEYWORD)),
    "expected puts-as-keyword at (4,0,4): {tokens:?}"
  );
  assert!(
    tokens.contains(&(0, 0, 3, TOKEN_KEYWORD)),
    "expected def-as-keyword at (0,0,3): {tokens:?}"
  );
  assert!(
    tokens.contains(&(2, 0, 3, TOKEN_KEYWORD)),
    "expected end-as-keyword at (2,0,3): {tokens:?}"
  );

  drop(client);
  handle.join().unwrap();
}

#[test]
fn classes_em_tags_counter_as_class_at_both_declaration_and_type_annotation_use() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///classes.em", CLASSES_SRC);

  let tokens = semantic_tokens(&client, "file:///classes.em");

  // "class Counter" is line 0; "Counter" starts at character 6.
  assert!(CLASSES_SRC.lines().next() == Some("class Counter"));
  assert!(
    tokens.contains(&(0, 6, 7, TOKEN_CLASS)),
    "expected Counter-as-class at its declaration (0,6,7): {tokens:?}"
  );

  // "c: Counter = Counter.new(10)" is line 16; the type-annotation
  // "Counter" starts at character 3.
  assert!(CLASSES_SRC.lines().nth(16) == Some("c: Counter = Counter.new(10)"));
  assert!(
    tokens.contains(&(16, 3, 7, TOKEN_CLASS)),
    "expected Counter-as-class at its type-annotation use (16,3,7): {tokens:?}"
  );

  drop(client);
  handle.join().unwrap();
}
