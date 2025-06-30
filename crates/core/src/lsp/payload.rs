use std::borrow::Cow;

use jsonrpsee_types::{Id, NotificationSer, RequestSer};
use lsp_types::{
    notification::{DidChangeTextDocument, DidOpenTextDocument, Initialized, Notification},
    request::{Initialize, Request as RequestTrait},
    CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializedParams, PartialResultParams, Position, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams,
};
use serde::Serialize;
use serde_json::value::RawValue;
use uuid::Uuid;

fn generate_uuid() -> Id<'static> {
    Id::Str(Cow::from(Uuid::new_v4().to_string()))
}

/// LspPayload implements functions to serialize a JSON RPC
/// request into a LSP request payload.
pub trait LspPayload: Serialize {
    fn to_payload(&mut self) -> anyhow::Result<String> {
        let content = serde_json::to_string(&self)?;
        let content_length = content.len();
        let payload = format!("Content-Length: {content_length}\r\n\r\n{content}");

        Ok(payload)
    }
}

impl LspPayload for RequestSer<'_> {}

impl LspPayload for NotificationSer<'_> {}

/// Create an LSP request.
/// Note that params is leaked here. Make sure to call to_payload to clean up.
fn create_request<T: Serialize>(
    method: &'static str,
    params: T,
) -> anyhow::Result<RequestSer<'static>> {
    let id = generate_uuid();
    let params_str = serde_json::to_string(&params)?;
    let params_raw = RawValue::from_string(params_str)?;

    let request = RequestSer::owned(id, Cow::from(method), Some(params_raw));
    Ok(request)
}

/// Create an LSP notification.
/// Note that params is leaked here. Make sure to call to_payload to clean up.
fn create_notification<T: Serialize>(
    method: &'static str,
    params: T,
) -> anyhow::Result<NotificationSer<'static>> {
    let params_str = serde_json::to_string(&params)?;
    let params_raw = RawValue::from_string(params_str)?;

    let notification = NotificationSer::owned(Cow::from(method), Some(params_raw));
    Ok(notification)
}

/// Create an initialize request.
pub fn initialize(options: Option<serde_json::Value>) -> anyhow::Result<RequestSer<'static>> {
    let params = InitializeParams {
        initialization_options: options,
        ..Default::default()
    };

    create_request(Initialize::METHOD, params)
}

/// Create an initialized notification.
pub fn initialized() -> anyhow::Result<NotificationSer<'static>> {
    let params = InitializedParams {};
    create_notification(Initialized::METHOD, params)
}

/// Create a textDocument/didOpen notification.
/// Hardcoded for sql.
pub fn did_open(uri: Url, text: &str) -> anyhow::Result<NotificationSer<'static>> {
    let params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri,
            language_id: "sql".to_string(),
            version: 1,
            text: text.to_string(),
        },
    };

    create_notification(DidOpenTextDocument::METHOD, params)
}

/// Create a textDocument/didChange notification.
/// Text should be all the text in the REPL (no range changes).
pub fn did_change(uri: Url, version: i32, text: &str) -> anyhow::Result<NotificationSer<'static>> {
    let params = DidChangeTextDocumentParams {
        text_document: VersionedTextDocumentIdentifier { uri, version },
        content_changes: vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: text.to_string(),
        }],
    };

    create_notification(DidChangeTextDocument::METHOD, params)
}

/// Create a textDocument/completion request.
/// Zero-indexed.
pub fn completion(uri: Url, line: u32, offset: u32) -> anyhow::Result<RequestSer<'static>> {
    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position {
                line,
                character: offset,
            },
        },
        work_done_progress_params: WorkDoneProgressParams {
            ..Default::default()
        },
        partial_result_params: PartialResultParams {
            ..Default::default()
        },
        context: None,
    };

    create_request("textDocument/completion", params)
}
