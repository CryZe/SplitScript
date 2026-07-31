//! Typed incoming LSP/JSON-RPC payloads used by the request router.

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct IncomingMessage {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TextDocumentPositionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TextDocumentParams {
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TextDocumentItem {
    pub uri: String,
    pub version: Option<i64>,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DidOpenParams {
    pub text_document: TextDocumentItem,
}

#[derive(Debug, Deserialize)]
pub(super) struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContentChange {
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DidChangeParams {
    pub text_document: VersionedTextDocumentIdentifier,
    pub content_changes: Vec<ContentChange>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodeActionContext {
    #[serde(default)]
    pub only: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodeActionParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    pub context: CodeActionContext,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InlayHintParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReferenceContext {
    #[serde(default)]
    pub include_declaration: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReferenceParams {
    #[serde(flatten)]
    pub position: TextDocumentPositionParams,
    pub context: ReferenceContext,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RenameParams {
    #[serde(flatten)]
    pub position: TextDocumentPositionParams,
    pub new_name: String,
}

pub(super) fn decode<T: DeserializeOwned>(params: Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|error| format!("invalid request parameters: {error}"))
}
