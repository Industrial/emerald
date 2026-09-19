//! Real end-to-end proof of plan 21's `leaf-go-to-definition`
//! acceptance criteria — drives `emerald_lsp::run` over
//! `Connection::memory()`, never calling its internal helpers
//! directly.

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::notification::{DidOpenTextDocument, Initialized, Notification as _};
use lsp_types::request::{GotoDefinition, Initialize, Request as _};
use lsp_types::{
  DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, InitializeParams,
  InitializedParams, PartialResultParams, Position, TextDocumentIdentifier, TextDocumentItem,
  TextDocumentPositionParams, WorkDoneProgressParams,
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
  client.receiver.recv().unwrap(); // the InitializeResult response
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
  // Every `didOpen` also publishes diagnostics (plan 17) — drain that
  // notification before sending the next real request.
  client.receiver.recv().unwrap();
}

fn goto_definition(
  client: &Connection,
  uri: &str,
  line: u32,
  character: u32,
) -> Option<GotoDefinitionResponse> {
  let params = GotoDefinitionParams {
    text_document_position_params: TextDocumentPositionParams {
      text_document: TextDocumentIdentifier {
        uri: uri.parse().unwrap(),
      },
      position: Position::new(line, character),
    },
    work_done_progress_params: WorkDoneProgressParams::default(),
    partial_result_params: PartialResultParams::default(),
  };
  client
    .sender
    .send(Message::Request(Request::new(
      RequestId::from(2),
      GotoDefinition::METHOD.to_string(),
      params,
    )))
    .unwrap();
  match client.receiver.recv().unwrap() {
    Message::Response(resp) => serde_json::from_value(resp.response_result.unwrap()).unwrap(),
    other => panic!("expected a Response, got {other:?}"),
  }
}

const HELLO_SRC: &str = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
const CLASSES_SRC: &str = "class Counter\n  value: Int64\n\n  fn initialize(start: Int64): Void do\n    @value = start\n  end\n\n  fn value: Int64 do\n    @value\n  end\n\n  fn add(n: Int64): Int64 do\n    @value + n\n  end\nend\n\nc: Counter = Counter.new(10)\nputs c.value\nputs c.add(5)\n\nclass Point\n  x: Float64\n  y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn sum: Float64 do\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

#[test]
fn definition_of_add_at_its_call_site_finds_the_real_declaration() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///hello.em", HELLO_SRC);

  // "add" inside "puts add(20, 22)" — line 4, cursor mid-word.
  let response = goto_definition(&client, "file:///hello.em", 4, 6);

  assert_eq!(HELLO_SRC.find("fn add"), Some(0));
  let Some(GotoDefinitionResponse::Scalar(loc)) = response else {
    panic!("expected a scalar Location, got {response:?}");
  };
  // "fn add" -> "add" starts at byte/char offset 3 on line 0.
  assert_eq!(loc.range.start, Position::new(0, 3));

  drop(client);
  handle.join().unwrap();
}

#[test]
fn definition_of_point_finds_the_second_class_not_the_first() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///classes.em", CLASSES_SRC);

  // "Point" inside "p: Point = Point.new(2.0, 3.0)" — the type
  // annotation occurrence, line 34, "Point" starts at character 3.
  let response = goto_definition(&client, "file:///classes.em", 34, 5);

  let Some(GotoDefinitionResponse::Scalar(loc)) = response else {
    panic!("expected a scalar Location, got {response:?}");
  };
  // "class Point" is the SECOND class declaration, on line 20 (0-based).
  assert_eq!(loc.range.start, Position::new(20, 6));

  drop(client);
  handle.join().unwrap();
}

#[test]
fn definition_on_a_method_call_target_returns_nothing_not_the_wrong_class() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///classes.em", CLASSES_SRC);

  // "value" inside "puts c.value" (line 17) — a method call, not a
  // top-level name; must fail safe, never guess which class's method.
  let response = goto_definition(&client, "file:///classes.em", 17, 8);
  assert_eq!(response, None);

  drop(client);
  handle.join().unwrap();
}

#[test]
fn definition_on_a_keyword_returns_nothing() {
  let (client, handle) = start_server();
  initialize(&client);
  did_open(&client, "file:///t.em", "if true do\n  puts 1\nend\n");

  let response = goto_definition(&client, "file:///t.em", 0, 0);
  assert_eq!(response, None);

  drop(client);
  handle.join().unwrap();
}
