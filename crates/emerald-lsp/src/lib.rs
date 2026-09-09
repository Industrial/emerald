//! Emerald language server (plan 17's `leaf-lsp-server`, extended by
//! plan 21's `leaf-go-to-definition`/`leaf-completion`/
//! `leaf-semantic-tokens`) — synchronous, channel-based on
//! `lsp-server`/`lsp-types` (the same scaffold rust-analyzer itself
//! uses), not `tower-lsp`/`async-lsp` — no async runtime needed for a
//! per-request parse-then-check call (see plan 17's Decision log, the
//! same reasoning plan 14 applied to `emerald-cli`).
//!
//! Full-document `textDocumentSync` only, no incremental diffing —
//! there is no incremental compiler to sync incrementally against.
//! Go-to-definition/completion/semantic-tokens are scoped to
//! top-level names only (functions/classes/modules), single-document,
//! no rename/formatting — see plan 21's own Decision log for exactly
//! what's deliberately out of scope (methods, in particular: this
//! server never resolves a method-call target, since that needs real
//! expression-level type inference this plan doesn't add).
//!
//! `run` is exposed here (not just inlined in `main.rs`) so integration
//! tests can drive it directly over `lsp_server::Connection::memory()`.

use lsp_server::{Connection, Message, Notification as ServerNotification, Response};
use lsp_types::request::{
  Completion, GotoDefinition, Request as LspRequest, SemanticTokensFullRequest,
};
use lsp_types::{
  CompletionItem, CompletionOptions, CompletionParams, CompletionResponse, Diagnostic,
  DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams,
  GotoDefinitionResponse, Location, OneOf, Position, PublishDiagnosticsParams, Range,
  SemanticToken, SemanticTokenType, SemanticTokens, SemanticTokensFullOptions,
  SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
  SemanticTokensServerCapabilities, ServerCapabilities, TextDocumentSyncCapability,
  TextDocumentSyncKind,
  notification::{DidChangeTextDocument, DidOpenTextDocument, Notification, PublishDiagnostics},
};
use std::collections::HashMap;

/// Plan 17's Decision log's own ground-truth terminal list (the same
/// set its TextMate grammar uses) — reused verbatim here for
/// completion and semantic-token keyword tagging rather than a second,
/// independently-drifting copy.
const KEYWORDS: &[&str] = &[
  "class", "module", "def", "end", "if", "else", "while", "return", "break", "next", "puts",
  "raise", "begin", "rescue", "new", "Array", "Proc",
];

fn semantic_tokens_legend() -> SemanticTokensLegend {
  SemanticTokensLegend {
    token_types: vec![
      SemanticTokenType::KEYWORD,
      SemanticTokenType::TYPE,
      SemanticTokenType::CLASS,
      SemanticTokenType::FUNCTION,
      SemanticTokenType::VARIABLE,
      SemanticTokenType::NUMBER,
    ],
    token_modifiers: vec![],
  }
}

const TOKEN_KEYWORD: u32 = 0;
const TOKEN_CLASS: u32 = 2;
const TOKEN_FUNCTION: u32 = 3;
const TOKEN_NUMBER: u32 = 5;

pub fn server_capabilities() -> ServerCapabilities {
  ServerCapabilities {
    text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
    definition_provider: Some(OneOf::Left(true)),
    completion_provider: Some(CompletionOptions::default()),
    semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
      SemanticTokensOptions {
        legend: semantic_tokens_legend(),
        full: Some(SemanticTokensFullOptions::Bool(true)),
        ..Default::default()
      },
    )),
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

  // Plan 21: go-to-definition/completion/semantic-tokens all need the
  // requested document's *current* text, which a request (unlike a
  // `didOpen`/`didChange` notification) never carries itself — kept
  // here, updated by `handle_notification`, keyed by the URI's own
  // string form (sidesteps needing `lsp_types::Uri: Hash`).
  let mut documents: HashMap<String, String> = HashMap::new();

  for msg in &connection.receiver {
    match msg {
      Message::Request(req) => {
        if connection.handle_shutdown(&req)? {
          break;
        }
        handle_request(connection, req, &documents)?;
      }
      Message::Notification(not) => handle_notification(connection, not, &mut documents)?,
      Message::Response(_) => {}
    }
  }
  Ok(())
}

fn handle_notification(
  connection: &Connection,
  not: ServerNotification,
  documents: &mut HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  match not.method.as_str() {
    DidOpenTextDocument::METHOD => {
      let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
      documents.insert(
        params.text_document.uri.as_str().to_string(),
        params.text_document.text.clone(),
      );
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
        documents.insert(
          params.text_document.uri.as_str().to_string(),
          change.text.clone(),
        );
        publish(connection, params.text_document.uri, &change.text)?;
      }
    }
    _ => {}
  }
  Ok(())
}

fn handle_request(
  connection: &Connection,
  req: lsp_server::Request,
  documents: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
  // Cloned to a plain owned `String` so the match arms below can move
  // `req` (into `req.extract`) while still holding a live `&str`
  // borrow of the method name for the match guard — `req.method`
  // itself can't be both borrowed and moved at once.
  let method = req.method.clone();
  match method.as_str() {
    m if m == <GotoDefinition as LspRequest>::METHOD => {
      let (id, params): (_, GotoDefinitionParams) = req.extract(m)?;
      let position = params.text_document_position_params.position;
      let uri = params.text_document_position_params.text_document.uri;
      let text = documents.get(uri.as_str()).cloned().unwrap_or_default();
      let table = emerald_driver::symbols(&text, "buffer.em").unwrap_or_default();
      let response = goto_definition(&text, uri, position, &table);
      connection.sender.send(Message::Response(Response::new_ok(
        id,
        serde_json::to_value(response)?,
      )))?;
    }
    m if m == <Completion as LspRequest>::METHOD => {
      let (id, params): (_, CompletionParams) = req.extract(m)?;
      let uri = params.text_document_position.text_document.uri;
      let text = documents.get(uri.as_str()).cloned().unwrap_or_default();
      let response: CompletionResponse = completion_items(&text).into();
      connection.sender.send(Message::Response(Response::new_ok(
        id,
        serde_json::to_value(Some(response))?,
      )))?;
    }
    m if m == <SemanticTokensFullRequest as LspRequest>::METHOD => {
      let (id, params): (_, SemanticTokensParams) = req.extract(m)?;
      let text = documents
        .get(params.text_document.uri.as_str())
        .cloned()
        .unwrap_or_default();
      let table = emerald_driver::symbols(&text, "buffer.em").unwrap_or_default();
      let tokens = SemanticTokens {
        result_id: None,
        data: semantic_tokens_data(&text, &table),
      };
      let response = SemanticTokensResult::Tokens(tokens);
      connection.sender.send(Message::Response(Response::new_ok(
        id,
        serde_json::to_value(Some(response))?,
      )))?;
    }
    // Any other request isn't handled yet — no response at all, same
    // as any real LSP server for a method it doesn't advertise.
    _ => {}
  }
  Ok(())
}

/// Plan 21's Decision log: go-to-definition is a word-boundary-
/// anchored TEXT search over the raw document, not real span
/// tracking (neither the parser's AST nor `emerald-sema` carry
/// declaration-site positions) — scoped to top-level names only.
/// `word` naming a class/module/function is looked up via a
/// `\b(?:def|class|module)\s+word\b`-shaped scan; anything else
/// (a keyword, a method name, an unresolved identifier) returns
/// `None` rather than guessing wrong.
fn goto_definition(
  text: &str,
  uri: lsp_types::Uri,
  position: Position,
  table: &emerald_driver::SymbolTable,
) -> Option<GotoDefinitionResponse> {
  let word = word_at_position(text, position)?;
  let keyword = if let Some(info) = table.classes.get(&word) {
    if info.is_module { "module" } else { "class" }
  } else if table.functions.contains_key(&word) {
    "def"
  } else {
    return None;
  };
  let start = find_declaration(text, keyword, &word)?;
  let end = start + word.len();
  let range = Range {
    start: offset_to_position(text, start),
    end: offset_to_position(text, end),
  };
  Some(GotoDefinitionResponse::Scalar(Location { uri, range }))
}

/// Finds the first `\b<keyword>\s+<name>\b` occurrence in `text`,
/// returning `name`'s own byte offset (not the keyword's) — a plain
/// hand-rolled scan rather than a `regex` dependency, since the
/// pattern shape needed here is fixed and simple.
fn find_declaration(text: &str, keyword: &str, name: &str) -> Option<usize> {
  let bytes = text.as_bytes();
  let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
  let mut search_from = 0usize;
  while let Some(rel) = text[search_from..].find(keyword) {
    let kw_start = search_from + rel;
    let kw_end = kw_start + keyword.len();
    let boundary_before = kw_start == 0 || !is_word(bytes[kw_start - 1]);
    let boundary_after_kw = kw_end >= bytes.len() || !is_word(bytes[kw_end]);
    if boundary_before && boundary_after_kw {
      let mut i = kw_end;
      while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
      }
      if text[i..].starts_with(name) {
        let name_end = i + name.len();
        let boundary_after_name = name_end >= bytes.len() || !is_word(bytes[name_end]);
        if boundary_after_name {
          return Some(i);
        }
      }
    }
    search_from = kw_end;
  }
  None
}

/// Extracts the identifier touching `position` — the character right
/// at the cursor, or the one immediately before it (a cursor placed
/// right after the last character of a word, the common "click at the
/// end" case).
fn word_at_position(text: &str, position: Position) -> Option<String> {
  let offset = position_to_offset(text, position)?;
  let bytes = text.as_bytes();
  let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
  let anchor = if offset < bytes.len() && is_word(bytes[offset]) {
    offset
  } else if offset > 0 && is_word(bytes[offset - 1]) {
    offset - 1
  } else {
    return None;
  };
  let mut start = anchor;
  while start > 0 && is_word(bytes[start - 1]) {
    start -= 1;
  }
  let mut end = anchor;
  while end < bytes.len() && is_word(bytes[end]) {
    end += 1;
  }
  Some(text[start..end].to_string())
}

/// The inverse of `offset_to_position` — converts an LSP `Position`
/// (UTF-16 line/character) back to a byte offset. `None` if `position`
/// names a line past the end of `text`.
fn position_to_offset(text: &str, position: Position) -> Option<usize> {
  let line_start = if position.line == 0 {
    0
  } else {
    let mut current_line = 0u32;
    let mut found = None;
    for (i, b) in text.as_bytes().iter().enumerate() {
      if *b == b'\n' {
        current_line += 1;
        if current_line == position.line {
          found = Some(i + 1);
          break;
        }
      }
    }
    found?
  };
  let line_rest = &text[line_start..];
  let mut utf16_count = 0u32;
  for (byte_idx, ch) in line_rest.char_indices() {
    if utf16_count >= position.character {
      return Some(line_start + byte_idx);
    }
    utf16_count += ch.len_utf16() as u32;
  }
  Some(line_start + line_rest.len())
}

/// Plan 21's Decision log: a flat concatenation of the reserved-
/// keyword list and the buffer's own top-level symbol names — no
/// ranking, no fuzzy matching. On an unparseable buffer,
/// `emerald_driver::symbols` failing degrades to the keyword-only
/// list rather than erroring.
fn completion_items(text: &str) -> Vec<CompletionItem> {
  let mut items: Vec<CompletionItem> = KEYWORDS
    .iter()
    .map(|k| CompletionItem::new_simple((*k).to_string(), "keyword".to_string()))
    .collect();
  if let Ok(table) = emerald_driver::symbols(text, "buffer.em") {
    for name in table.functions.keys() {
      items.push(CompletionItem::new_simple(
        name.clone(),
        "function".to_string(),
      ));
    }
    for name in table.classes.keys() {
      items.push(CompletionItem::new_simple(
        name.clone(),
        "class".to_string(),
      ));
    }
  }
  items
}

/// Plan 21's Decision log: a line-by-line lexical pass (the same
/// categories the plan-17 TextMate grammar already covers), additionally
/// cross-referencing every bare identifier against `table` to tag a
/// known class name `class` and a known function name `function`
/// wherever it occurs — declaration or use alike. Returned already
/// delta-encoded (line/character both relative to the previous token),
/// the LSP spec's own required wire format.
fn semantic_tokens_data(text: &str, table: &emerald_driver::SymbolTable) -> Vec<SemanticToken> {
  let mut raw: Vec<(u32, u32, u32, u32)> = Vec::new();
  for (line_idx, line) in text.lines().enumerate() {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let mut utf16_col = 0u32;
    while i < chars.len() {
      let c = chars[i];
      if c.is_ascii_alphabetic() || c == '_' {
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
          i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let width = word.encode_utf16().count() as u32;
        let token_type = if KEYWORDS.contains(&word.as_str()) {
          Some(TOKEN_KEYWORD)
        } else if table.classes.contains_key(&word) {
          Some(TOKEN_CLASS)
        } else if table.functions.contains_key(&word) {
          Some(TOKEN_FUNCTION)
        } else {
          None
        };
        if let Some(tt) = token_type {
          raw.push((line_idx as u32, utf16_col, width, tt));
        }
        utf16_col += width;
      } else if c.is_ascii_digit() {
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
          i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let width = word.encode_utf16().count() as u32;
        raw.push((line_idx as u32, utf16_col, width, TOKEN_NUMBER));
        utf16_col += width;
      } else {
        utf16_col += c.len_utf16() as u32;
        i += 1;
      }
    }
  }

  delta_encode(raw)
}

/// The LSP spec's own required wire format: each token's `line`/
/// `character` is relative to the *previous* token, not absolute.
fn delta_encode(raw: Vec<(u32, u32, u32, u32)>) -> Vec<SemanticToken> {
  let mut tokens = Vec::with_capacity(raw.len());
  let mut prev_line = 0u32;
  let mut prev_char = 0u32;
  for (line, character, length, token_type) in raw {
    let delta_line = line - prev_line;
    let delta_start = if delta_line == 0 {
      character - prev_char
    } else {
      character
    };
    tokens.push(SemanticToken {
      delta_line,
      delta_start,
      length,
      token_type,
      token_modifiers_bitset: 0,
    });
    prev_line = line;
    prev_char = character;
  }
  tokens
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
      .map(|d| Diagnostic {
        range: Range {
          start: offset_to_position(text, d.span.0),
          end: offset_to_position(text, d.span.1.max(d.span.0 + 1)),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        message: d.message.clone(),
        ..Default::default()
      })
      .collect(),
    // `check()` only ever parses + type-checks — codegen/link/require
    // (plan 49's graph-building errors, only ever produced by
    // `emerald_driver::parallel::compile_parallel`) are unreachable
    // here, but the match must stay exhaustive.
    Err(
      emerald_driver::DriverError::Codegen(_)
      | emerald_driver::DriverError::Link(_)
      | emerald_driver::DriverError::Require(_),
    ) => {
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
