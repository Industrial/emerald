//! Emerald language server (plan 17's `leaf-lsp-server`) — synchronous,
//! channel-based on `lsp-server`/`lsp-types` (the same scaffold
//! rust-analyzer itself uses), not `tower-lsp`/`async-lsp` — no async
//! runtime needed for a per-request parse-then-check call (see plan
//! 17's Decision log, the same reasoning plan 14 applied to
//! `emerald-cli`).
//!
//! Full-document `textDocumentSync` only, no incremental diffing —
//! there is no incremental compiler to sync incrementally against.
//! Diagnostics only: no semantic tokens, go-to-definition, completion,
//! rename, or formatting — `emerald-sema` exposes no symbol table yet.
//!
//! `run` is exposed here (not just inlined in `main.rs`) so integration
//! tests can drive it directly over `lsp_server::Connection::memory()`.

use lsp_server::{Connection, Message, Notification as ServerNotification};
use lsp_types::{
  Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams, Position,
  PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
  TextDocumentSyncKind,
  notification::{DidChangeTextDocument, DidOpenTextDocument, Notification, PublishDiagnostics},
};

pub fn server_capabilities() -> ServerCapabilities {
  ServerCapabilities {
    text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
    ..Default::default()
  }
}

/// The server's main loop: performs the `initialize` handshake, then
/// dispatches `didOpen`/`didChange` (both re-check the full buffer and
/// publish diagnostics) until `shutdown`/`exit` or the connection
/// closes. Works identically over `Connection::stdio()` (`main.rs`)
/// and `Connection::memory()` (tests).
pub fn run(connection: &Connection) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  let caps = serde_json::to_value(server_capabilities())?;
  connection.initialize(caps)?;

  for msg in &connection.receiver {
    match msg {
      Message::Request(req) => {
        if connection.handle_shutdown(&req)? {
          break;
        }
        // No other requests are handled yet — this is a diagnostics-
        // only server (see module doc comment).
      }
      Message::Notification(not) => handle_notification(connection, not)?,
      Message::Response(_) => {}
    }
  }
  Ok(())
}

fn handle_notification(
  connection: &Connection,
  not: ServerNotification,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  match not.method.as_str() {
    DidOpenTextDocument::METHOD => {
      let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
      publish(
        connection,
        params.text_document.uri,
        &params.text_document.text,
      )?;
    }
    DidChangeTextDocument::METHOD => {
      let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
      // Full sync (`server_capabilities` above): exactly one change
      // event, its `text` the whole new document, `range: None`.
      if let Some(change) = params.content_changes.into_iter().next() {
        publish(connection, params.text_document.uri, &change.text)?;
      }
    }
    _ => {}
  }
  Ok(())
}

fn publish(
  connection: &Connection,
  uri: lsp_types::Uri,
  text: &str,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  let params = PublishDiagnosticsParams {
    uri,
    diagnostics: check_diagnostics(text),
    version: None,
  };
  let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
  connection
    .sender
    .send(Message::Notification(notification))?;
  Ok(())
}

/// Runs `emerald_driver::check` on `text` and converts the result into
/// LSP diagnostics — `Ok(())` is an empty `Vec` (clears any previously
/// published diagnostics for this document, doesn't just skip
/// publishing).
pub fn check_diagnostics(text: &str) -> Vec<Diagnostic> {
  match emerald_driver::check(text, "buffer.em") {
    Ok(()) => vec![],
    Err(emerald_driver::DriverError::Parse(errs)) => {
      errs.iter().map(|e| parse_diagnostic(e, text)).collect()
    }
    Err(emerald_driver::DriverError::Sema(diags)) => diags
      .iter()
      .map(|d| whole_document_diagnostic(text, &d.message))
      .collect(),
    // `check()` only ever parses + type-checks — codegen/link are
    // unreachable here, but the match must stay exhaustive.
    Err(emerald_driver::DriverError::Codegen(_) | emerald_driver::DriverError::Link(_)) => {
      vec![]
    }
  }
}

/// `emerald_parser::ParseError` has no public fields — its span is
/// reached only through the `miette::Diagnostic` trait's `.labels()`,
/// generic over `E` so this crate never needs to name `ParseError`
/// directly (mirrors `emerald-mcp`'s identical approach).
fn parse_diagnostic<E: miette::Diagnostic + std::fmt::Display>(e: &E, text: &str) -> Diagnostic {
  let message = e.to_string();
  let (start, end) = e
    .labels()
    .and_then(|mut labels| labels.next())
    .map(|label| (label.offset(), label.offset() + label.len().max(1)))
    .unwrap_or((0, 0));
  Diagnostic {
    range: Range {
      start: offset_to_position(text, start),
      end: offset_to_position(text, end),
    },
    severity: Some(DiagnosticSeverity::ERROR),
    message,
    ..Default::default()
  }
}

/// `emerald_sema::Diagnostic` carries no span at all (a real,
/// pre-existing, disclosed gap — plan 13's own deferral) — anchored at
/// the whole document instead of fabricating a precise range.
fn whole_document_diagnostic(text: &str, message: &str) -> Diagnostic {
  Diagnostic {
    range: Range {
      start: Position::new(0, 0),
      end: end_of_document(text),
    },
    severity: Some(DiagnosticSeverity::ERROR),
    message: message.to_string(),
    ..Default::default()
  }
}

fn end_of_document(text: &str) -> Position {
  let last_line = text.lines().count().saturating_sub(1) as u32;
  let last_line_text = text.lines().last().unwrap_or("");
  Position::new(last_line, utf16_len(last_line_text))
}

/// Converts a byte offset to an LSP `Position` — LSP columns are
/// UTF-16 code units, not bytes.
fn offset_to_position(text: &str, offset: usize) -> Position {
  let offset = offset.min(text.len());
  let mut line = 0u32;
  let mut last_newline = None;
  for (i, b) in text.as_bytes()[..offset].iter().enumerate() {
    if *b == b'\n' {
      line += 1;
      last_newline = Some(i + 1);
    }
  }
  let line_start = last_newline.unwrap_or(0);
  let column = utf16_len(&text[line_start..offset]);
  Position::new(line, column)
}

fn utf16_len(s: &str) -> u32 {
  s.encode_utf16().count() as u32
}
