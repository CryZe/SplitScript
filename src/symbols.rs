//! Editor-neutral document outline symbols.

use crate::{
    ast::{Program, SettingDecl, SettingKind, Span},
    lexer::TokenKind,
    syntax::SourceDocument,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSymbolKind {
    Namespace,
    Struct,
    Field,
    Enum,
    EnumVariant,
    Function,
    Method,
    Variable,
    Property,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: DocumentSymbolKind,
    pub range: Span,
    pub selection_range: Span,
    pub children: Vec<DocumentSymbol>,
}

pub fn document_symbols(document: &SourceDocument, program: &Program) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    if let Some(state) = &program.state {
        let selection = identifier_in(document, state.span, "state").unwrap_or(state.span);
        let mut children = state
            .fields
            .iter()
            .map(|field| DocumentSymbol {
                name: field.name.clone(),
                detail: Some("state field".to_owned()),
                kind: DocumentSymbolKind::Field,
                range: field.span,
                selection_range: identifier_in(document, field.span, &field.name)
                    .unwrap_or(field.span),
                children: Vec::new(),
            })
            .collect::<Vec<_>>();
        sort_symbols(&mut children);
        symbols.push(DocumentSymbol {
            name: "state".to_owned(),
            detail: Some(state.processes.join(", ")),
            kind: DocumentSymbolKind::Namespace,
            range: state.span,
            selection_range: selection,
            children,
        });
    }

    if let Some((range, selection)) = keyword_block(document, "settings") {
        symbols.push(DocumentSymbol {
            name: "settings".to_owned(),
            detail: None,
            kind: DocumentSymbolKind::Namespace,
            range,
            selection_range: selection,
            children: setting_symbols(document, &program.settings),
        });
    }

    symbols.extend(program.globals.iter().map(|global| DocumentSymbol {
        name: global.name.clone(),
        detail: Some("global".to_owned()),
        kind: DocumentSymbolKind::Variable,
        range: global.span,
        selection_range: identifier_in(document, global.span, &global.name).unwrap_or(global.span),
        children: Vec::new(),
    }));

    symbols.extend(program.records.iter().map(|record| {
        DocumentSymbol {
            name: record.name.clone(),
            detail: None,
            kind: DocumentSymbolKind::Struct,
            range: record.span,
            selection_range: identifier_in(document, record.span, &record.name)
                .unwrap_or(record.span),
            children: record
                .fields
                .iter()
                .map(|field| DocumentSymbol {
                    name: field.name.clone(),
                    detail: None,
                    kind: DocumentSymbolKind::Field,
                    range: field.span,
                    selection_range: identifier_in(document, field.span, &field.name)
                        .unwrap_or(field.span),
                    children: Vec::new(),
                })
                .collect(),
        }
    }));

    symbols.extend(program.enums.iter().map(|enumeration| {
        DocumentSymbol {
            name: enumeration.name.clone(),
            detail: None,
            kind: DocumentSymbolKind::Enum,
            range: enumeration.span,
            selection_range: identifier_in(document, enumeration.span, &enumeration.name)
                .unwrap_or(enumeration.span),
            children: enumeration
                .variants
                .iter()
                .map(|variant| DocumentSymbol {
                    name: variant.name.clone(),
                    detail: variant.payload.map(|_| "payload variant".to_owned()),
                    kind: DocumentSymbolKind::EnumVariant,
                    range: variant.span,
                    selection_range: identifier_in(document, variant.span, &variant.name)
                        .unwrap_or(variant.span),
                    children: Vec::new(),
                })
                .collect(),
        }
    }));

    symbols.extend(program.functions.iter().map(|function| {
        DocumentSymbol {
            name: function.name.clone(),
            detail: function
                .method_of
                .map(|_| "method".to_owned())
                .or_else(|| Some("function".to_owned())),
            kind: if function.method_of.is_some() {
                DocumentSymbolKind::Method
            } else {
                DocumentSymbolKind::Function
            },
            range: function.span,
            selection_range: identifier_in(document, function.span, &function.name)
                .unwrap_or(function.span),
            children: Vec::new(),
        }
    }));

    symbols.extend(program.actions.iter().map(|action| DocumentSymbol {
        name: action.kind.name().to_owned(),
        detail: Some("lifecycle block".to_owned()),
        kind: DocumentSymbolKind::Event,
        range: action.span,
        selection_range:
            identifier_in(document, action.span, action.kind.name()).unwrap_or(action.span),
        children: Vec::new(),
    }));

    sort_symbols(&mut symbols);
    symbols
}

fn setting_symbols(document: &SourceDocument, settings: &[SettingDecl]) -> Vec<DocumentSymbol> {
    fn one(
        document: &SourceDocument,
        settings: &[SettingDecl],
        cursor: &mut usize,
    ) -> DocumentSymbol {
        let setting = &settings[*cursor];
        *cursor += 1;
        let is_title = matches!(setting.kind, SettingKind::Title { .. });
        let mut symbol = DocumentSymbol {
            name: if is_title {
                setting.description.clone()
            } else {
                setting.name.clone()
            },
            detail: Some(
                match setting.kind {
                    SettingKind::Bool { .. } => "bool setting",
                    SettingKind::Title { .. } => "settings group",
                    SettingKind::Choice { .. } => "choice setting",
                    SettingKind::File { .. } => "file setting",
                }
                .to_owned(),
            ),
            kind: if is_title {
                DocumentSymbolKind::Namespace
            } else {
                DocumentSymbolKind::Property
            },
            range: setting.span,
            selection_range: if is_title {
                string_in(document, setting.span, &setting.description).unwrap_or(setting.span)
            } else {
                identifier_in(document, setting.span, &setting.name).unwrap_or(setting.span)
            },
            children: Vec::new(),
        };
        if is_title {
            while *cursor < settings.len() && contains(setting.span, settings[*cursor].span) {
                symbol.children.push(one(document, settings, cursor));
            }
        }
        symbol
    }

    let mut cursor = 0;
    let mut symbols = Vec::new();
    while cursor < settings.len() {
        symbols.push(one(document, settings, &mut cursor));
    }
    symbols
}

fn identifier_in(document: &SourceDocument, span: Span, name: &str) -> Option<Span> {
    document.tokens().find_map(|token| {
        (span.start <= token.span.start
            && token.span.end <= span.end
            && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name))
        .then_some(token.span)
    })
}

fn string_in(document: &SourceDocument, span: Span, value: &str) -> Option<Span> {
    document.tokens().find_map(|token| {
        (span.start <= token.span.start
            && token.span.end <= span.end
            && matches!(&token.kind, TokenKind::String(spelling) if spelling == value))
        .then_some(token.span)
    })
}

fn keyword_block(document: &SourceDocument, keyword: &str) -> Option<(Span, Span)> {
    let tokens = document.tokens().collect::<Vec<_>>();
    let keyword_index = tokens.windows(2).position(|pair| {
        matches!(&pair[0].kind, TokenKind::Ident(name) if name == keyword)
            && pair[1].kind == TokenKind::LBrace
    })?;
    let opening = keyword_index + 1;
    let mut depth = 0usize;
    for token in &tokens[opening..] {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some((
                        tokens[keyword_index].span.join(token.span),
                        tokens[keyword_index].span,
                    ));
                }
            }
            _ => {}
        }
    }
    Some((
        tokens[keyword_index].span.join(tokens.last()?.span),
        tokens[keyword_index].span,
    ))
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn sort_symbols(symbols: &mut [DocumentSymbol]) {
    symbols.sort_by_key(|symbol| (symbol.range.start, symbol.range.end));
}
