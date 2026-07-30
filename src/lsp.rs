//! Language Server Protocol state backed by [`crate::database::CompilerDatabase`].
//!
//! Transport framing lives in the `splitls` binary. Keeping message handling
//! here makes the protocol behavior directly testable and leaves compiler
//! queries independent of JSON-RPC.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    Diagnostic, DiagnosticSeverity, FixApplicability,
    ast::Span,
    completion::{CompletionItem, CompletionKind, CompletionList},
    database::{CompilerDatabase, DefinitionTarget},
    highlight::{SEMANTIC_TOKEN_MODIFIERS, SemanticHighlight, SemanticTokenKind},
    insight::{HoverInfo, SignatureHelp},
    symbols::{DocumentSymbol, DocumentSymbolKind},
};

struct Document {
    version: Option<i64>,
    database: CompilerDatabase,
}

/// Stateful single-process LSP handler for open SplitScript documents.
#[derive(Default)]
pub struct LanguageServer {
    documents: HashMap<String, Document>,
    initialized: bool,
    shutdown_requested: bool,
    should_exit: bool,
}

impl LanguageServer {
    /// Handles one decoded JSON-RPC message and returns zero or more outgoing
    /// responses and notifications.
    pub fn handle(&mut self, message: Value) -> Vec<Value> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Vec::new();
        };
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        if !self.initialized && !matches!(method, "initialize" | "exit") {
            return id.map_or_else(Vec::new, |id| {
                vec![error_response(id, -32002, "server is not initialized")]
            });
        }

        if self.shutdown_requested && method != "exit" {
            return id.map_or_else(Vec::new, |id| {
                vec![error_response(id, -32600, "server has shut down")]
            });
        }

        match method {
            "initialize" => {
                self.initialized = true;
                id.map_or_else(Vec::new, |id| {
                    vec![response(
                        id,
                        json!({
                            "capabilities": {
                                "positionEncoding": "utf-16",
                                "textDocumentSync": {
                                    "openClose": true,
                                    "change": 1
                                },
                                "documentFormattingProvider": true,
                                "documentSymbolProvider": true,
                                "codeActionProvider": {
                                    "codeActionKinds": ["quickfix"],
                                    "resolveProvider": false
                                },
                                "completionProvider": {
                                    "resolveProvider": false,
                                    "triggerCharacters": ["."]
                                },
                                "hoverProvider": true,
                                "definitionProvider": true,
                                "referencesProvider": true,
                                "renameProvider": {
                                    "prepareProvider": true
                                },
                                "signatureHelpProvider": {
                                    "triggerCharacters": ["(", ","],
                                    "retriggerCharacters": [","]
                                },
                                "semanticTokensProvider": {
                                    "legend": {
                                        "tokenTypes": SemanticTokenKind::ALL
                                            .iter()
                                            .map(|kind| kind.name())
                                            .collect::<Vec<_>>(),
                                        "tokenModifiers": SEMANTIC_TOKEN_MODIFIERS
                                    },
                                    "full": true
                                }
                            },
                            "serverInfo": {
                                "name": "splitls",
                                "version": env!("CARGO_PKG_VERSION")
                            }
                        }),
                    )]
                })
            }
            "initialized" => Vec::new(),
            "shutdown" => {
                self.shutdown_requested = true;
                id.map_or_else(Vec::new, |id| vec![response(id, Value::Null)])
            }
            "exit" => {
                self.should_exit = true;
                Vec::new()
            }
            "textDocument/didOpen" => self.did_open(&params),
            "textDocument/didChange" => self.did_change(&params),
            "textDocument/didClose" => self.did_close(&params),
            "textDocument/formatting" => {
                id.map_or_else(Vec::new, |id| vec![self.formatting_response(id, &params)])
            }
            "textDocument/documentSymbol" => id.map_or_else(Vec::new, |id| {
                vec![self.document_symbol_response(id, &params)]
            }),
            "textDocument/codeAction" => {
                id.map_or_else(Vec::new, |id| vec![self.code_action_response(id, &params)])
            }
            "textDocument/semanticTokens/full" => id.map_or_else(Vec::new, |id| {
                vec![self.semantic_tokens_response(id, &params)]
            }),
            "textDocument/completion" => {
                id.map_or_else(Vec::new, |id| vec![self.completion_response(id, &params)])
            }
            "textDocument/hover" => {
                id.map_or_else(Vec::new, |id| vec![self.hover_response(id, &params)])
            }
            "textDocument/definition" => {
                id.map_or_else(Vec::new, |id| vec![self.definition_response(id, &params)])
            }
            "textDocument/references" => {
                id.map_or_else(Vec::new, |id| vec![self.references_response(id, &params)])
            }
            "textDocument/prepareRename" => id.map_or_else(Vec::new, |id| {
                vec![self.prepare_rename_response(id, &params)]
            }),
            "textDocument/rename" => {
                id.map_or_else(Vec::new, |id| vec![self.rename_response(id, &params)])
            }
            "textDocument/signatureHelp" => id.map_or_else(Vec::new, |id| {
                vec![self.signature_help_response(id, &params)]
            }),
            _ if id.is_some() => vec![error_response(
                id.unwrap(),
                -32601,
                format!("method `{method}` is not supported"),
            )],
            _ => Vec::new(),
        }
    }

    pub const fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn did_open(&mut self, params: &Value) -> Vec<Value> {
        let Some(text_document) = params.get("textDocument") else {
            return Vec::new();
        };
        let (Some(uri), Some(text)) = (
            text_document.get("uri").and_then(Value::as_str),
            text_document.get("text").and_then(Value::as_str),
        ) else {
            return Vec::new();
        };
        let uri = uri.to_owned();
        self.documents.insert(
            uri.clone(),
            Document {
                version: text_document.get("version").and_then(Value::as_i64),
                database: CompilerDatabase::new(text),
            },
        );
        vec![self.diagnostics_notification(&uri)]
    }

    fn did_change(&mut self, params: &Value) -> Vec<Value> {
        let Some(text_document) = params.get("textDocument") else {
            return Vec::new();
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(text) = params
            .get("contentChanges")
            .and_then(Value::as_array)
            .and_then(|changes| changes.last())
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        let Some(document) = self.documents.get_mut(uri) else {
            return Vec::new();
        };
        document.version = text_document.get("version").and_then(Value::as_i64);
        document.database.set_source(text);
        vec![self.diagnostics_notification(uri)]
    }

    fn did_close(&mut self, params: &Value) -> Vec<Value> {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Vec::new();
        };
        self.documents.remove(uri);
        vec![json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": []
            }
        })]
    }

    fn formatting_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return error_response(id, -32602, "missing text document URI");
        };
        let Some(document) = self.documents.get_mut(uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let Ok(formatted) = document.database.format() else {
            return response(id, json!([]));
        };
        if *formatted == source {
            return response(id, json!([]));
        }
        response(
            id,
            json!([{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": position(&source, source.len())
                },
                "newText": &*formatted
            }]),
        )
    }

    fn document_symbol_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return error_response(id, -32602, "missing text document URI");
        };
        let Some(document) = self.documents.get_mut(uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let Ok(symbols) = document.database.document_symbols() else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                symbols
                    .iter()
                    .map(|symbol| document_symbol_json(&source, symbol))
                    .collect(),
            ),
        )
    }

    fn code_action_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return error_response(id, -32602, "missing text document URI");
        };
        if params
            .pointer("/context/only")
            .and_then(Value::as_array)
            .is_some_and(|kinds| {
                !kinds.is_empty()
                    && !kinds.iter().any(|kind| {
                        kind.as_str()
                            .is_some_and(|kind| kind == "quickfix" || kind.starts_with("quickfix."))
                    })
            })
        {
            return response(id, json!([]));
        }
        let Some(document) = self.documents.get_mut(&uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let (Some(start), Some(end)) = (
            params
                .pointer("/range/start")
                .and_then(|position| offset_from_json_position(&source, position)),
            params
                .pointer("/range/end")
                .and_then(|position| offset_from_json_position(&source, position)),
        ) else {
            return error_response(id, -32602, "invalid code-action range");
        };
        let diagnostics = document.database.diagnostics();
        let mut actions = Vec::new();
        for diagnostic in diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.span.start <= end && start <= diagnostic.span.end)
        {
            for fix in &diagnostic.fixes {
                let edits = fix
                    .edits
                    .iter()
                    .map(|edit| {
                        json!({
                            "range": {
                                "start": position(&source, edit.span.start),
                                "end": position(&source, edit.span.end)
                            },
                            "newText": edit.replacement
                        })
                    })
                    .collect::<Vec<_>>();
                let mut changes = serde_json::Map::new();
                changes.insert(uri.clone(), Value::Array(edits));
                actions.push(json!({
                    "title": fix.title,
                    "kind": "quickfix",
                    "diagnostics": [diagnostic_json(&uri, &source, diagnostic)],
                    "isPreferred": fix.applicability == FixApplicability::MachineApplicable,
                    "edit": { "changes": changes },
                    "data": { "applicability": fix.applicability.to_string() }
                }));
            }
        }
        response(id, Value::Array(actions))
    }

    fn diagnostics_notification(&mut self, uri: &str) -> Value {
        let Some(document) = self.documents.get_mut(uri) else {
            return Value::Null;
        };
        let diagnostics = document.database.diagnostics();
        let source = document.database.source();
        let diagnostics = diagnostics
            .iter()
            .map(|diagnostic| diagnostic_json(uri, source, diagnostic))
            .collect::<Vec<_>>();
        let mut params = json!({
            "uri": uri,
            "diagnostics": diagnostics
        });
        if let Some(version) = document.version {
            params["version"] = json!(version);
        }
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": params
        })
    }

    fn semantic_tokens_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return error_response(id, -32602, "missing text document URI");
        };
        let Some(document) = self.documents.get_mut(uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let Ok(highlights) = document.database.semantic_highlights() else {
            return response(id, json!({ "data": [] }));
        };
        response(
            id,
            json!({
                "data": semantic_token_data(
                    document.database.source(),
                    highlights.highlights()
                )
            }),
        )
    }

    fn completion_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return error_response(id, -32602, "missing text document URI");
        };
        let (Some(line), Some(character)) = (
            params.pointer("/position/line").and_then(Value::as_u64),
            params
                .pointer("/position/character")
                .and_then(Value::as_u64),
        ) else {
            return error_response(id, -32602, "missing completion position");
        };
        let Some(document) = self.documents.get_mut(uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let Some(offset) = offset_at_position(&source, line as u32, character as u32) else {
            return error_response(id, -32602, "completion position is outside the document");
        };
        let Ok(completions) = document.database.completions(offset) else {
            return response(id, json!({ "isIncomplete": false, "items": [] }));
        };
        response(id, completion_list_json(&source, &completions))
    }

    fn hover_response(&mut self, id: Value, params: &Value) -> Value {
        let Some((source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid hover document or position");
        };
        let Ok(hover) = document.database.hover(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            hover.map_or(Value::Null, |hover| hover_json(&source, &hover)),
        )
    }

    fn definition_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return error_response(id, -32602, "missing text document URI");
        };
        let Some((source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid definition document or position");
        };
        let Ok(definition) = document.database.definition_at(offset) else {
            return response(id, Value::Null);
        };
        let location = match definition {
            Some(DefinitionTarget::Source(definition)) => {
                location_json(&uri, &source, definition.span)
            }
            Some(
                DefinitionTarget::StandardLibrary(_)
                | DefinitionTarget::StandardLibrarySymbol(_)
                | DefinitionTarget::Language(_),
            )
            | None => Value::Null,
        };
        response(id, location)
    }

    fn references_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return error_response(id, -32602, "missing text document URI");
        };
        let include_declaration = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some((source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid references document or position");
        };
        let Ok(references) = document.database.references_at(offset, include_declaration) else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                references
                    .into_iter()
                    .map(|span| location_json(&uri, &source, span))
                    .collect(),
            ),
        )
    }

    fn prepare_rename_response(&mut self, id: Value, params: &Value) -> Value {
        let Some((source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid rename document or position");
        };
        let Ok(target) = document.database.rename_target_at(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            target.map_or(Value::Null, |target| {
                json!({
                    "range": {
                        "start": position(&source, target.span.start),
                        "end": position(&source, target.span.end)
                    },
                    "placeholder": target.name
                })
            }),
        )
    }

    fn rename_response(&mut self, id: Value, params: &Value) -> Value {
        let Some(uri) = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return error_response(id, -32602, "missing text document URI");
        };
        let Some(new_name) = params
            .get("newName")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return error_response(id, -32602, "missing rename identifier");
        };
        let Some((source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid rename document or position");
        };
        let spans = match document.database.rename_at(offset, &new_name) {
            Ok(spans) => spans,
            Err(error) => return error_response(id, -32602, error.to_string()),
        };
        let edits = spans
            .into_iter()
            .map(|span| {
                json!({
                    "range": {
                        "start": position(&source, span.start),
                        "end": position(&source, span.end)
                    },
                    "newText": new_name
                })
            })
            .collect();
        let mut changes = serde_json::Map::new();
        changes.insert(uri, Value::Array(edits));
        response(id, json!({ "changes": changes }))
    }

    fn signature_help_response(&mut self, id: Value, params: &Value) -> Value {
        let Some((_source, offset, document)) = self.document_at_position(params) else {
            return error_response(id, -32602, "invalid signature-help document or position");
        };
        let Ok(help) = document.database.signature_help(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            help.map_or(Value::Null, |help| signature_help_json(&help)),
        )
    }

    fn document_at_position<'a>(
        &'a mut self,
        params: &Value,
    ) -> Option<(String, usize, &'a mut Document)> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?;
        let line = params.pointer("/position/line")?.as_u64()? as u32;
        let character = params.pointer("/position/character")?.as_u64()? as u32;
        let document = self.documents.get_mut(uri)?;
        let source = document.database.source().to_owned();
        let offset = offset_at_position(&source, line, character)?;
        Some((source, offset, document))
    }
}

fn response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

fn diagnostic_json(uri: &str, source: &str, diagnostic: &Diagnostic) -> Value {
    let mut message = diagnostic.message.clone();
    for note in &diagnostic.notes {
        message.push_str("\n\nnote: ");
        message.push_str(note);
    }
    let related_information = diagnostic
        .labels
        .iter()
        .filter_map(|label| {
            Some(json!({
                "location": {
                    "uri": uri,
                    "range": {
                        "start": position(source, label.span.start),
                        "end": position(source, label.span.end)
                    }
                },
                "message": label.message.as_ref()?
            }))
        })
        .collect::<Vec<_>>();
    let fixes = diagnostic
        .fixes
        .iter()
        .map(|fix| {
            json!({
                "title": fix.title,
                "applicability": fix.applicability.to_string(),
                "edits": fix.edits.iter().map(|edit| json!({
                    "range": {
                        "start": position(source, edit.span.start),
                        "end": position(source, edit.span.end)
                    },
                    "newText": edit.replacement
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "range": {
            "start": position(source, diagnostic.span.start),
            "end": position(source, diagnostic.span.end)
        },
        "severity": match diagnostic.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Information => 3,
            DiagnosticSeverity::Hint => 4,
        },
        "code": diagnostic.code.as_str(),
        "source": "splitscript",
        "message": message,
        "relatedInformation": related_information,
        "data": { "fixes": fixes }
    })
}

fn location_json(uri: &str, source: &str, span: Span) -> Value {
    json!({
        "uri": uri,
        "range": {
            "start": position(source, span.start),
            "end": position(source, span.end)
        }
    })
}

fn document_symbol_json(source: &str, symbol: &DocumentSymbol) -> Value {
    let mut value = json!({
        "name": symbol.name,
        "kind": document_symbol_kind_number(symbol.kind),
        "range": {
            "start": position(source, symbol.range.start),
            "end": position(source, symbol.range.end)
        },
        "selectionRange": {
            "start": position(source, symbol.selection_range.start),
            "end": position(source, symbol.selection_range.end)
        },
        "children": symbol.children.iter()
            .map(|child| document_symbol_json(source, child))
            .collect::<Vec<_>>()
    });
    if let Some(detail) = &symbol.detail {
        value["detail"] = json!(detail);
    }
    value
}

const fn document_symbol_kind_number(kind: DocumentSymbolKind) -> u32 {
    match kind {
        DocumentSymbolKind::Namespace => 3,
        DocumentSymbolKind::Struct => 23,
        DocumentSymbolKind::Field => 8,
        DocumentSymbolKind::Enum => 10,
        DocumentSymbolKind::EnumVariant => 22,
        DocumentSymbolKind::Function => 12,
        DocumentSymbolKind::Method => 6,
        DocumentSymbolKind::Variable => 13,
        DocumentSymbolKind::Property => 7,
        DocumentSymbolKind::Event => 24,
    }
}

fn offset_from_json_position(source: &str, position: &Value) -> Option<usize> {
    offset_at_position(
        source,
        position.get("line")?.as_u64()? as u32,
        position.get("character")?.as_u64()? as u32,
    )
}

fn position(source: &str, offset: usize) -> Value {
    let (line, character) = position_parts(source, offset);
    json!({
        "line": line,
        "character": character
    })
}

fn position_parts(source: &str, offset: usize) -> (u32, u32) {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_text = before.rsplit_once('\n').map_or(before, |(_, tail)| tail);
    (line, line_text.encode_utf16().count() as u32)
}

fn semantic_token_data(source: &str, highlights: &[SemanticHighlight]) -> Vec<u32> {
    let mut absolute = Vec::<(u32, u32, u32, u32, u32)>::new();
    for highlight in highlights {
        let mut start = highlight.span.start.min(source.len());
        let end = highlight.span.end.min(source.len());
        while start < end {
            let rest = &source[start..end];
            let line_end = rest.find('\n').map_or(end, |relative| start + relative);
            let visible_end = if line_end > start && source.as_bytes()[line_end - 1] == b'\r' {
                line_end - 1
            } else {
                line_end
            };
            if start < visible_end {
                let (line, character) = position_parts(source, start);
                let length = source[start..visible_end].encode_utf16().count() as u32;
                absolute.push((
                    line,
                    character,
                    length,
                    highlight.kind.index(),
                    highlight.modifiers,
                ));
            }
            if line_end == end {
                break;
            }
            start = line_end + 1;
        }
    }
    absolute.sort_by_key(|token| (token.0, token.1));

    let mut data = Vec::with_capacity(absolute.len() * 5);
    let (mut previous_line, mut previous_start) = (0, 0);
    for (line, start, length, kind, modifiers) in absolute {
        let delta_line = line - previous_line;
        let delta_start = if delta_line == 0 {
            start - previous_start
        } else {
            start
        };
        data.extend_from_slice(&[delta_line, delta_start, length, kind, modifiers]);
        previous_line = line;
        previous_start = start;
    }
    data
}

fn offset_at_position(source: &str, target_line: u32, target_character: u32) -> Option<usize> {
    let line_start = if target_line == 0 {
        0
    } else {
        let mut remaining = target_line;
        let mut start = None;
        for (offset, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                remaining -= 1;
                if remaining == 0 {
                    start = Some(offset + 1);
                    break;
                }
            }
        }
        start?
    };
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |relative| line_start + relative);
    let mut utf16 = 0u32;
    for (relative, character) in source[line_start..line_end].char_indices() {
        if utf16 == target_character {
            return Some(line_start + relative);
        }
        utf16 += character.len_utf16() as u32;
        if utf16 > target_character {
            return None;
        }
    }
    (utf16 == target_character).then_some(line_end)
}

fn completion_list_json(source: &str, completions: &CompletionList) -> Value {
    json!({
        "isIncomplete": false,
        "items": completions
            .items
            .iter()
            .map(|item| completion_item_json(source, completions, item))
            .collect::<Vec<_>>()
    })
}

fn completion_item_json(
    source: &str,
    completions: &CompletionList,
    item: &CompletionItem,
) -> Value {
    let mut completion = json!({
        "label": item.label,
        "kind": completion_kind_number(item.kind),
        "insertTextFormat": if item.is_snippet { 2 } else { 1 },
        "textEdit": {
            "range": {
                "start": position(source, completions.replacement.start),
                "end": position(source, completions.replacement.end)
            },
            "newText": item.insert_text
        }
    });
    if let Some(detail) = &item.detail {
        completion["detail"] = json!(detail);
    }
    if let Some(documentation) = &item.documentation {
        completion["documentation"] = json!({
            "kind": "markdown",
            "value": documentation
        });
    }
    completion
}

const fn completion_kind_number(kind: CompletionKind) -> u32 {
    match kind {
        CompletionKind::Keyword => 14,
        CompletionKind::Snippet => 15,
        CompletionKind::Namespace => 9,
        CompletionKind::Function => 3,
        CompletionKind::Method => 2,
        CompletionKind::Variable => 6,
        CompletionKind::Setting | CompletionKind::Property => 10,
        CompletionKind::StateField => 5,
        CompletionKind::Type => 7,
        CompletionKind::Struct => 22,
        CompletionKind::Enum => 13,
        CompletionKind::EnumMember => 20,
    }
}

fn hover_json(source: &str, hover: &HoverInfo) -> Value {
    json!({
        "contents": {
            "kind": "markdown",
            "value": hover.markdown
        },
        "range": {
            "start": position(source, hover.span.start),
            "end": position(source, hover.span.end)
        }
    })
}

fn signature_help_json(help: &SignatureHelp) -> Value {
    json!({
        "signatures": help.signatures.iter().map(|signature| json!({
            "label": signature.label,
            "documentation": {
                "kind": "markdown",
                "value": signature.documentation
            },
            "parameters": signature.parameters.iter().map(|parameter| json!({
                "label": parameter.label,
                "documentation": {
                    "kind": "markdown",
                    "value": parameter.documentation
                }
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "activeSignature": help.active_signature,
        "activeParameter": help.active_parameter
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "method": method, "params": params })
    }

    fn initialize(server: &mut LanguageServer) {
        server.handle(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {}
        }));
    }

    #[test]
    fn advertises_full_sync_diagnostics_formatting_and_semantic_tokens() {
        let mut server = LanguageServer::default();
        let response = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }));
        assert_eq!(response[0]["id"], 1);
        assert_eq!(
            response[0]["result"]["capabilities"]["textDocumentSync"]["change"],
            1
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["documentFormattingProvider"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["documentSymbolProvider"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["codeActionProvider"]["codeActionKinds"][0],
            "quickfix"
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["semanticTokensProvider"]["full"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["completionProvider"]["triggerCharacters"][0],
            "."
        );
        assert_eq!(response[0]["result"]["capabilities"]["hoverProvider"], true);
        assert_eq!(
            response[0]["result"]["capabilities"]["definitionProvider"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["referencesProvider"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["renameProvider"]["prepareProvider"],
            true
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["signatureHelpProvider"]["triggerCharacters"],
            json!(["(", ","])
        );
        assert_eq!(
            response[0]["result"]["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]
                [SemanticTokenKind::StateField.index() as usize],
            "stateField"
        );

        let diagnostics = server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///game.split",
                    "languageId": "splitscript",
                    "version": 4,
                    "text": "state \"game.exe\" {"
                }
            }),
        ));
        assert_eq!(diagnostics[0]["method"], "textDocument/publishDiagnostics");
        assert_eq!(diagnostics[0]["params"]["version"], 4);
        assert_eq!(diagnostics[0]["params"]["diagnostics"][0]["code"], "SS0002");
    }

    #[test]
    fn changes_reuse_the_document_database_and_formatting_ignores_type_errors() {
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///game.split",
                    "version": 1,
                    "text": "state \"game.exe\" {}"
                }
            }),
        ));
        let diagnostics = server.handle(notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///game.split", "version": 2 },
                "contentChanges": [{
                    "text": "state \"game.exe\"{}\nwhileAttached{let broken:bool=42}"
                }]
            }),
        ));
        assert_eq!(diagnostics[0]["params"]["version"], 2);
        assert_eq!(diagnostics[0]["params"]["diagnostics"][0]["code"], "SS0003");

        let formatting = server.handle(json!({
            "jsonrpc": "2.0",
            "id": "format",
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": "file:///game.split" },
                "options": { "tabSize": 4, "insertSpaces": true }
            }
        }));
        assert_eq!(formatting[0]["id"], "format");
        assert_eq!(formatting[0]["result"].as_array().unwrap().len(), 1);
        assert!(
            formatting[0]["result"][0]["newText"]
                .as_str()
                .unwrap()
                .contains("let broken: bool = 42")
        );
    }

    #[test]
    fn positions_use_utf16_code_units_and_close_clears_diagnostics() {
        assert_eq!(
            position("🦊x", "🦊".len()),
            json!({ "line": 0, "character": 2 })
        );

        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///game.split",
                    "version": 1,
                    "text": "state \"game.exe\" {"
                }
            }),
        ));
        let closed = server.handle(notification(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": "file:///game.split" } }),
        ));
        assert!(
            closed[0]["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn diagnostic_conversion_preserves_notes_labels_and_fixes() {
        use crate::ast::Span;

        let diagnostic = Diagnostic::new("bad value", Span { start: 5, end: 6 })
            .with_secondary_label(Span { start: 0, end: 4 }, "declared here")
            .with_note("values must agree")
            .with_machine_applicable_fix("replace it", Span { start: 5, end: 6 }, "0");
        let converted = diagnostic_json("file:///game.split", "🦊\nvalue", &diagnostic);

        assert!(
            converted["message"]
                .as_str()
                .unwrap()
                .contains("note: values must agree")
        );
        assert_eq!(
            converted["relatedInformation"][0]["message"],
            "declared here"
        );
        assert_eq!(converted["data"]["fixes"][0]["title"], "replace it");
        assert_eq!(converted["data"]["fixes"][0]["edits"][0]["newText"], "0");
    }

    #[test]
    fn shutdown_requires_a_following_exit_notification() {
        let mut server = LanguageServer::default();
        initialize(&mut server);
        let response = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "shutdown"
        }));
        assert_eq!(response[0]["result"], Value::Null);
        assert!(!server.should_exit());
        server.handle(notification("exit", Value::Null));
        assert!(server.should_exit());
    }

    #[test]
    fn semantic_tokens_cover_language_domains_and_use_utf16_deltas() {
        let source = concat!(
            "// 🦊\n",
            "enum Mode { Active }\n",
            "state \"game.exe\" { level = process.read.i32(0) }\n",
            "settings { \"General\" { \"Enabled\" => enabled: true } }\n",
            "debug fn inspect(mode: Mode) { debug print(mode as String) }\n",
            "whileAttached {\n",
            "    let marker = await process.scan(0, 1, sig\"48 ??\")\n",
            "    if current.level == 1 { inspect(Mode.Active) }\n",
            "}\n"
        );
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///semantic.split",
                    "version": 1,
                    "text": source
                }
            }),
        ));
        let response = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "textDocument/semanticTokens/full",
            "params": { "textDocument": { "uri": "file:///semantic.split" } }
        }));
        let data = response[0]["result"]["data"]
            .as_array()
            .expect("semantic token data");
        assert_eq!(data.len() % 5, 0);
        let kinds = data
            .chunks_exact(5)
            .map(|token| token[3].as_u64().unwrap() as u32)
            .collect::<Vec<_>>();
        for expected in [
            SemanticTokenKind::SettingTitle,
            SemanticTokenKind::Setting,
            SemanticTokenKind::StateField,
            SemanticTokenKind::Lifecycle,
            SemanticTokenKind::Enum,
            SemanticTokenKind::EnumMember,
            SemanticTokenKind::Signature,
            SemanticTokenKind::Debug,
        ] {
            assert!(
                kinds.contains(&expected.index()),
                "missing {expected:?} semantic token"
            );
        }
        assert!(data.chunks_exact(5).any(|token| {
            token[4].as_u64().unwrap() as u32 & crate::highlight::MODIFIER_DEBUG != 0
        }));

        assert_eq!(position_parts("🦊x", "🦊".len()), (0, 2));
    }

    #[test]
    fn completion_uses_inferred_members_catalog_docs_and_utf16_text_edits() {
        let source = concat!(
            "// 🦊\n",
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let number: i32 = 4\n",
            "    number.cl\n",
            "}\n"
        );
        let offset = source.find("number.cl").unwrap() + "number.cl".len();
        let (line, character) = position_parts(source, offset);
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///completion.split",
                    "version": 1,
                    "text": source
                }
            }),
        ));
        let response = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "file:///completion.split" },
                "position": { "line": line, "character": character }
            }
        }));
        let items = response[0]["result"]["items"]
            .as_array()
            .expect("completion items");
        let clamp = items
            .iter()
            .find(|item| item["label"] == "clamp")
            .expect("numeric clamp completion");
        assert_eq!(clamp["kind"], 2);
        assert_eq!(clamp["insertTextFormat"], 2);
        assert_eq!(
            clamp["textEdit"]["newText"],
            "clamp(${1:minimum}, ${2:maximum})"
        );
        assert_eq!(
            clamp["textEdit"]["range"]["start"],
            json!({ "line": line, "character": character - 2 })
        );
        assert!(
            clamp["documentation"]["value"]
                .as_str()
                .unwrap()
                .contains("smaller")
                || clamp["documentation"]["value"]
                    .as_str()
                    .unwrap()
                    .contains("inclusive range")
        );

        assert_eq!(offset_at_position("🦊x", 0, 2), Some("🦊".len()));
        assert_eq!(offset_at_position("🦊x", 0, 1), None);
    }

    #[test]
    fn hover_and_signature_help_preserve_resolved_catalog_information() {
        let source = concat!(
            "// 🦊\n",
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let number: i32 = 4\n",
            "    let bounded = number.clamp(0, 7)\n",
            "}\n"
        );
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///insight.split",
                    "version": 1,
                    "text": source
                }
            }),
        ));

        let hover_offset = source.find("clamp").unwrap() + 2;
        let (hover_line, hover_character) = position_parts(source, hover_offset);
        let hover = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///insight.split" },
                "position": { "line": hover_line, "character": hover_character }
            }
        }));
        let markdown = hover[0]["result"]["contents"]["value"]
            .as_str()
            .expect("hover markdown");
        assert!(markdown.contains("i32.clamp"));
        assert!(markdown.contains("T = i32"));
        assert!(markdown.contains("Runtime behavior"));
        assert!(markdown.contains("Examples"));

        let value_offset = source.rfind("number").unwrap() + 2;
        let (value_line, value_character) = position_parts(source, value_offset);
        let value_hover = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 90,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///insight.split" },
                "position": { "line": value_line, "character": value_character }
            }
        }));
        assert!(
            value_hover[0]["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("let number: i32")
        );
        assert_eq!(
            value_hover[0]["result"]["range"]["start"],
            position(source, source.rfind("number").unwrap())
        );

        let parameter_offset = source.find(", 7").unwrap() + 2;
        let (parameter_line, parameter_character) = position_parts(source, parameter_offset);
        let signature = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "textDocument/signatureHelp",
            "params": {
                "textDocument": { "uri": "file:///insight.split" },
                "position": { "line": parameter_line, "character": parameter_character }
            }
        }));
        assert_eq!(signature[0]["result"]["activeParameter"], 1);
        assert!(
            signature[0]["result"]["signatures"][0]["label"]
                .as_str()
                .unwrap()
                .starts_with("i32.clamp")
        );
        assert_eq!(
            signature[0]["result"]["signatures"][0]["parameters"][1]["label"],
            "maximum"
        );
    }

    #[test]
    fn catalog_docs_completion_and_hover_stay_in_sync() {
        use crate::{documentation::StandardLibraryDocumentation, stdlib::StdlibItemId};

        let incomplete = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let number: i32 = 4\n",
            "    number.cl\n",
            "}\n"
        );
        let generic = StandardLibraryDocumentation::generate(StdlibItemId::NumericClamp, &[]);
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": "file:///catalog-sync.split",
                    "version": 1,
                    "text": incomplete
                }
            }),
        ));
        let completion_offset = incomplete.find("number.cl").unwrap() + "number.cl".len();
        let (line, character) = position_parts(incomplete, completion_offset);
        let completion = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "file:///catalog-sync.split" },
                "position": { "line": line, "character": character }
            }
        }));
        let clamp = completion[0]["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "clamp")
            .expect("clamp completion");
        assert_eq!(clamp["detail"], generic.signature);
        assert_eq!(clamp["documentation"]["value"], generic.summary_markdown());

        let complete = incomplete.replace("number.cl\n", "number.clamp(0, 7)\n");
        server.handle(notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///catalog-sync.split", "version": 2 },
                "contentChanges": [{ "text": complete }]
            }),
        ));
        let hover_offset = complete.find("clamp").unwrap() + 2;
        let (line, character) = position_parts(&complete, hover_offset);
        let hover = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///catalog-sync.split" },
                "position": { "line": line, "character": character }
            }
        }));
        let resolved = StandardLibraryDocumentation::generate(
            StdlibItemId::NumericClamp,
            &[("T", "i32".to_owned())],
        );
        assert_eq!(
            hover[0]["result"]["contents"]["value"],
            resolved.hover_markdown()
        );
        assert_eq!(generic.summary_markdown(), resolved.summary_markdown());
    }

    #[test]
    fn definition_and_references_use_source_identities_and_utf16_ranges() {
        let source = concat!(
            "// 🦊\n",
            "state \"game.exe\" {}\n",
            "fn inspect(value: i32) { print(value as String) }\n",
            "whileAttached { inspect(1) }\n"
        );
        let uri = "file:///navigation.split";
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "version": 1,
                    "text": source
                }
            }),
        ));

        let call = source.rfind("inspect").unwrap() + 2;
        let (line, character) = position_parts(source, call);
        let definition = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        let declaration = source.find("inspect").unwrap();
        assert_eq!(definition[0]["result"]["uri"], uri);
        assert_eq!(
            definition[0]["result"]["range"]["start"],
            position(source, declaration)
        );
        assert_eq!(
            definition[0]["result"]["range"]["end"],
            position(source, declaration + "inspect".len())
        );

        let references = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": false }
            }
        }));
        assert_eq!(references[0]["result"].as_array().unwrap().len(), 1);
        assert_eq!(
            references[0]["result"][0]["range"]["start"],
            position(source, source.rfind("inspect").unwrap())
        );

        let with_declaration = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": true }
            }
        }));
        assert_eq!(with_declaration[0]["result"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn prepare_rename_and_rename_emit_validated_workspace_edits() {
        let source = concat!(
            "// \u{1f98a}\n",
            "state \"game.exe\" {}\n",
            "fn inspect(value: i32) { print(value as String) }\n",
            "whileAttached { inspect(1) }\n"
        );
        let uri = "file:///rename.split";
        let call = source.rfind("inspect").unwrap();
        let (line, character) = position_parts(source, call + 2);
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "version": 1,
                    "text": source
                }
            }),
        ));

        let prepared = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 16,
            "method": "textDocument/prepareRename",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        assert_eq!(prepared[0]["result"]["placeholder"], "inspect");
        assert_eq!(
            prepared[0]["result"]["range"]["start"],
            position(source, call)
        );

        let renamed = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 17,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "newName": "examine"
            }
        }));
        let edits = renamed[0]["result"]["changes"][uri]
            .as_array()
            .expect("workspace edits for the open URI");
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit["newText"] == "examine"));

        let reserved = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 18,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "newName": "while"
            }
        }));
        assert_eq!(reserved[0]["error"]["code"], -32602);
        assert!(
            reserved[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("reserved")
        );

        let print = source.find("print").unwrap();
        let (line, character) = position_parts(source, print + 1);
        let catalog = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 19,
            "method": "textDocument/prepareRename",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        assert_eq!(catalog[0]["result"], Value::Null);
    }

    #[test]
    fn document_symbols_and_code_actions_preserve_compiler_structure() {
        let symbols_source = concat!(
            "record Point { x: i32 }\n",
            "state \"game.exe\" { level = process.read.i32(0) }\n",
            "settings { \"General\" { \"Enabled\" => enabled: true } }\n",
            "whileAttached {}\n"
        );
        let symbols_uri = "file:///symbols.split";
        let mut server = LanguageServer::default();
        initialize(&mut server);
        server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": symbols_uri,
                    "version": 1,
                    "text": symbols_source
                }
            }),
        ));
        let symbols = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": symbols_uri } }
        }));
        let outline = symbols[0]["result"].as_array().unwrap();
        assert_eq!(
            outline
                .iter()
                .map(|symbol| symbol["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Point", "state", "settings", "whileAttached"]
        );
        assert_eq!(outline[0]["kind"], 23);
        assert_eq!(outline[0]["children"][0]["name"], "x");
        assert_eq!(outline[2]["children"][0]["name"], "General");
        assert_eq!(outline[2]["children"][0]["children"][0]["name"], "enabled");

        let broken = "state \"game.exe\" {}\nwhileAttached { let value: i32?? = None }\n";
        let broken_uri = "file:///fix.split";
        let diagnostics = server.handle(notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": broken_uri,
                    "version": 1,
                    "text": broken
                }
            }),
        ));
        assert_eq!(
            diagnostics[0]["params"]["diagnostics"][0]["data"]["fixes"][0]["title"],
            "remove the repeated postfix"
        );
        let actions = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "textDocument/codeAction",
            "params": {
                "textDocument": { "uri": broken_uri },
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end": position(broken, broken.len())
                },
                "context": {
                    "diagnostics": diagnostics[0]["params"]["diagnostics"],
                    "only": ["quickfix"]
                }
            }
        }));
        let quick_fixes = actions[0]["result"].as_array().unwrap();
        assert_eq!(quick_fixes.len(), 1);
        assert_eq!(quick_fixes[0]["title"], "remove the repeated postfix");
        assert_eq!(quick_fixes[0]["kind"], "quickfix");
        assert_eq!(quick_fixes[0]["isPreferred"], true);
        assert_eq!(
            quick_fixes[0]["edit"]["changes"][broken_uri][0]["newText"],
            ""
        );

        let unrelated = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "textDocument/codeAction",
            "params": {
                "textDocument": { "uri": broken_uri },
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": position(broken, broken.len())
                },
                "context": { "diagnostics": [], "only": ["source"] }
            }
        }));
        assert!(unrelated[0]["result"].as_array().unwrap().is_empty());
    }
}
