//! Conversion between compiler-owned byte-span products and LSP JSON values.

use serde_json::{Value, json};

use crate::{
    Diagnostic, DiagnosticSeverity,
    ast::Span,
    completion::{CompletionItem, CompletionKind, CompletionList},
    highlight::SemanticHighlight,
    inlay_hints::InlayHint,
    insight::{HoverInfo, SignatureHelp},
    symbols::{DocumentSymbol, DocumentSymbolKind},
    tooling::database::{DocumentHighlight, DocumentHighlightKind},
};

pub(super) fn document_highlight_json(source: &str, highlight: DocumentHighlight) -> Value {
    json!({
        "range": {
            "start": position(source, highlight.span.start),
            "end": position(source, highlight.span.end)
        },
        "kind": match highlight.kind {
            DocumentHighlightKind::Text => 1,
            DocumentHighlightKind::Read => 2,
            DocumentHighlightKind::Write => 3,
        }
    })
}

pub(super) fn inlay_hint_json(source: &str, hint: &InlayHint) -> Value {
    json!({
        "position": position(source, hint.position),
        "label": hint.label,
        "kind": 1
    })
}

pub(super) fn diagnostic_json(uri: &str, source: &str, diagnostic: &Diagnostic) -> Value {
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
    let mut value = json!({
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
    });
    if let Some(topic) = &diagnostic.migration_topic {
        value["codeDescription"] = json!({
            "href": format!(
                "splitscript-docs:{}",
                crate::documentation::migration_topic_uri(topic),
            )
        });
        value["data"]["migrationTopic"] = json!(topic);
    }
    value
}

pub(super) fn location_json(uri: &str, source: &str, span: Span) -> Value {
    json!({
        "uri": uri,
        "range": {
            "start": position(source, span.start),
            "end": position(source, span.end)
        }
    })
}

pub(super) fn selection_range_json(source: &str, spans: &[Span]) -> Value {
    let mut parent = Value::Null;
    for span in spans.iter().rev() {
        let mut selection = json!({
            "range": {
                "start": position(source, span.start),
                "end": position(source, span.end)
            }
        });
        if !parent.is_null() {
            selection["parent"] = parent;
        }
        parent = selection;
    }
    parent
}

pub(super) fn document_symbol_json(source: &str, symbol: &DocumentSymbol) -> Value {
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

pub(super) fn position(source: &str, offset: usize) -> Value {
    let (line, character) = position_parts(source, offset);
    json!({
        "line": line,
        "character": character
    })
}

pub(super) fn position_parts(source: &str, offset: usize) -> (u32, u32) {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_text = before.rsplit_once('\n').map_or(before, |(_, tail)| tail);
    (line, line_text.encode_utf16().count() as u32)
}

pub(super) fn semantic_token_data(source: &str, highlights: &[SemanticHighlight]) -> Vec<u32> {
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

pub(super) fn offset_at_position(
    source: &str,
    target_line: u32,
    target_character: u32,
) -> Option<usize> {
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

pub(super) fn completion_list_json(source: &str, completions: &CompletionList) -> Value {
    json!({
        "isIncomplete": false,
        "items": completions
            .items
            .iter()
            .map(|item| completion_item_json(source, completions, item))
            .collect::<Vec<_>>()
    })
}

pub(super) fn completion_item_json(
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
            "value": markdown_with_documentation_link(
                documentation,
                item.documentation_uri.as_deref(),
            )
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

pub(super) fn hover_json(source: &str, hover: &HoverInfo) -> Value {
    json!({
        "contents": {
            "kind": "markdown",
            "value": markdown_with_documentation_link(
                &hover.markdown,
                hover.documentation_uri.as_deref(),
            )
        },
        "range": {
            "start": position(source, hover.span.start),
            "end": position(source, hover.span.end)
        }
    })
}

fn markdown_with_documentation_link(markdown: &str, uri: Option<&str>) -> String {
    let Some(uri) = uri else {
        return markdown.to_owned();
    };
    let arguments = serde_json::to_string(&[uri])
        .expect("a documentation URI always serializes as command arguments");
    format!(
        "{markdown}\n\n[Open full documentation](command:splitscript.openDocumentation?{})",
        percent_encode_uri_component(arguments.as_bytes())
    )
}

fn percent_encode_uri_component(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(bytes.len());
    for byte in bytes.iter().copied() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

pub(super) fn signature_help_json(help: &SignatureHelp) -> Value {
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
