//! Type-directed completion and lexical recognition of type grammar positions.

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, catalog_language_completion,
    identifier_span, lexer, render_documentation,
};
use crate::{
    ast::{Program, Span},
    language::{LanguageCatalog, LanguageItemId},
    lexer::{Token, TokenKind},
    stdlib::{StandardLibrary, StdlibTypeKind, TypeConstructorSyntax},
};

pub(super) fn complete_type_position(
    source: &str,
    syntax: &Program,
    offset: usize,
    library: &StandardLibrary,
) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let lexed = lexer::lex_lossless(source).ok()?;
    let tokens = lexed
        .tokens()
        .filter(|token| !matches!(token.kind, TokenKind::DocComment(_) | TokenKind::Eof))
        .collect::<Vec<_>>();
    let prefix_end = tokens.partition_point(|token| token.span.end <= replacement.start);
    let prefix = &tokens[..prefix_end];

    let in_type_position = prefix.iter().enumerate().rev().any(|(index, token)| {
        let starts_type = matches!(&token.kind, TokenKind::Ident(name) if name == "as")
            || matches!(token.kind, TokenKind::Gt)
                && index > 0
                && matches!(prefix[index - 1].kind, TokenKind::Minus)
                && is_function_return_arrow(prefix, index - 1)
            || matches!(token.kind, TokenKind::Colon) && is_declaration_type_colon(prefix, index)
            || matches!(token.kind, TokenKind::LParen) && is_enum_payload_open(prefix, index);
        starts_type && type_prefix_expects_type(&prefix[index + 1..])
    });
    if !in_type_position {
        return None;
    }

    Some(build_type_completions(
        source,
        syntax,
        offset,
        replacement,
        library,
    ))
}

pub(super) fn complete_explicit_type_argument(
    source: &str,
    syntax: &Program,
    offset: usize,
    library: &StandardLibrary,
) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let before = &source[..replacement.start];
    let open = before.rfind('<')?;
    let name_end = open;
    let mut name_start = name_end;
    while name_start > 0 && super::is_identifier_byte(source.as_bytes()[name_start - 1]) {
        name_start -= 1;
    }
    if name_start == name_end {
        return None;
    }
    let name = &source[name_start..name_end];
    let is_generic_call = library
        .items()
        .iter()
        .any(|item| item.name == name && item.signature.explicit_type_parameters != 0);
    if !is_generic_call || source[open + 1..replacement.start].contains(['>', '(', ')', '{', '}']) {
        return None;
    }

    Some(build_type_completions(
        source,
        syntax,
        offset,
        replacement,
        library,
    ))
}

fn build_type_completions(
    source: &str,
    syntax: &Program,
    offset: usize,
    replacement: Span,
    library: &StandardLibrary,
) -> CompletionList {
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    add_type_completions(&mut builder, syntax, library);
    builder.finish()
}

pub(super) fn add_type_completions(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    library: &StandardLibrary,
) {
    let language = LanguageCatalog::new();
    for ty in library.core_types() {
        if let Some(item) = language.builtin_type(ty.id) {
            builder.add(catalog_language_completion(
                ty.name,
                CompletionKind::Type,
                item,
                ty.name.to_owned(),
                false,
            ));
        }
    }
    for ty in library.types() {
        builder.add(CompletionItem {
            label: ty.name.to_owned(),
            kind: match ty.kind {
                StdlibTypeKind::Intrinsic => CompletionKind::Type,
                StdlibTypeKind::Struct => CompletionKind::Struct,
                StdlibTypeKind::Enum => CompletionKind::Enum,
            },
            detail: Some("standard-library type".to_owned()),
            documentation: Some(render_documentation(&ty.documentation)),
            insert_text: ty.name.to_owned(),
            is_snippet: false,
        });
    }
    for constructor in library
        .type_constructors()
        .iter()
        .filter(|constructor| constructor.syntax == TypeConstructorSyntax::Named)
    {
        let parameters = constructor
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
            .collect::<Vec<_>>()
            .join(", ");
        builder.add(CompletionItem {
            label: constructor.name.to_owned(),
            kind: CompletionKind::Type,
            detail: Some(library.render_type_constructor(constructor.id)),
            documentation: Some(render_documentation(&constructor.documentation)),
            insert_text: format!("{}<{parameters}>", constructor.name),
            is_snippet: true,
        });
    }
    for record in &syntax.records {
        builder.add(CompletionItem {
            label: record.name.clone(),
            kind: CompletionKind::Struct,
            detail: Some("record type".to_owned()),
            documentation: record.documentation.clone(),
            insert_text: record.name.clone(),
            is_snippet: false,
        });
    }
    for enumeration in syntax.enum_declarations() {
        builder.add(CompletionItem {
            label: enumeration.name.clone(),
            kind: CompletionKind::Enum,
            detail: Some("enum type".to_owned()),
            documentation: enumeration.documentation.clone(),
            insert_text: enumeration.name.clone(),
            is_snippet: false,
        });
    }

    let array = language.item(LanguageItemId::ArrayType);
    for (label, insert_text) in [("[T]", "[${1:T}]"), ("[T; N]", "[${1:T}; ${2:length}]")] {
        builder.add(catalog_language_completion(
            label,
            CompletionKind::Type,
            array,
            insert_text.to_owned(),
            true,
        ));
    }
    let option = language.item(LanguageItemId::OptionType);
    builder.add(catalog_language_completion(
        "T?",
        CompletionKind::Type,
        option,
        "${1:T}?".to_owned(),
        true,
    ));
    let result = language.item(LanguageItemId::ResultType);
    builder.add(catalog_language_completion(
        "T!",
        CompletionKind::Type,
        result,
        "${1:T}!".to_owned(),
        true,
    ));
    let asynchronous = language.item(LanguageItemId::Async);
    builder.add(catalog_language_completion(
        "async T",
        CompletionKind::Type,
        asynchronous,
        "async ${1:T}".to_owned(),
        true,
    ));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypePrefix {
    Complete,
    ExpectsType,
    Incomplete,
    Invalid,
}

fn type_prefix_expects_type(tokens: &[&Token]) -> bool {
    let mut index = 0;
    matches!(
        parse_type_prefix(tokens, &mut index),
        TypePrefix::ExpectsType
    ) && index == tokens.len()
}

fn parse_type_prefix(tokens: &[&Token], index: &mut usize) -> TypePrefix {
    let Some(token) = tokens.get(*index) else {
        return TypePrefix::ExpectsType;
    };
    if matches!(&token.kind, TokenKind::Ident(name) if name == "async") {
        *index += 1;
        return parse_type_prefix(tokens, index);
    }

    match token.kind {
        TokenKind::Ident(_) => {
            *index += 1;
            if tokens
                .get(*index)
                .is_some_and(|token| matches!(token.kind, TokenKind::Lt))
            {
                *index += 1;
                loop {
                    match parse_type_prefix(tokens, index) {
                        TypePrefix::Complete => {}
                        other => return other,
                    }
                    let Some(delimiter) = tokens.get(*index) else {
                        return TypePrefix::Incomplete;
                    };
                    match delimiter.kind {
                        TokenKind::Comma => {
                            *index += 1;
                            if *index == tokens.len() {
                                return TypePrefix::ExpectsType;
                            }
                        }
                        TokenKind::Gt => {
                            *index += 1;
                            break;
                        }
                        _ => return TypePrefix::Invalid,
                    }
                }
            }
        }
        TokenKind::LBracket => {
            *index += 1;
            match parse_type_prefix(tokens, index) {
                TypePrefix::Complete => {}
                other => return other,
            }
            let Some(delimiter) = tokens.get(*index) else {
                return TypePrefix::Incomplete;
            };
            if matches!(delimiter.kind, TokenKind::Semicolon) {
                *index += 1;
                let Some(length) = tokens.get(*index) else {
                    return TypePrefix::Incomplete;
                };
                if !matches!(length.kind, TokenKind::Int(_)) {
                    return TypePrefix::Invalid;
                }
                *index += 1;
            }
            let Some(close) = tokens.get(*index) else {
                return TypePrefix::Incomplete;
            };
            if !matches!(close.kind, TokenKind::RBracket) {
                return TypePrefix::Invalid;
            }
            *index += 1;
        }
        _ => return TypePrefix::Invalid,
    }

    while tokens
        .get(*index)
        .is_some_and(|token| matches!(token.kind, TokenKind::Question | TokenKind::Bang))
    {
        *index += 1;
    }
    TypePrefix::Complete
}

fn is_declaration_type_colon(tokens: &[&Token], colon: usize) -> bool {
    let Some(name) = colon.checked_sub(1) else {
        return false;
    };
    if !matches!(tokens[name].kind, TokenKind::Ident(_)) {
        return false;
    }
    if name > 0 && matches!(&tokens[name - 1].kind, TokenKind::Ident(keyword) if keyword == "let") {
        return true;
    }
    if nearest_unclosed(tokens, colon, TokenKind::LParen, TokenKind::RParen)
        .is_some_and(|open| is_function_parameter_list(tokens, open))
    {
        return true;
    }
    nearest_unclosed(tokens, colon, TokenKind::LBrace, TokenKind::RBrace).is_some_and(|open| {
        is_named_declaration_body(tokens, open, "record")
            || is_named_declaration_body(tokens, open, "layout")
            || is_state_body(tokens, open)
    })
}

fn is_function_return_arrow(tokens: &[&Token], minus: usize) -> bool {
    let Some(close) = minus.checked_sub(1) else {
        return false;
    };
    if !matches!(tokens[close].kind, TokenKind::RParen) {
        return false;
    }
    matching_open(tokens, close, TokenKind::LParen, TokenKind::RParen)
        .is_some_and(|open| is_function_parameter_list(tokens, open))
}

fn is_enum_payload_open(tokens: &[&Token], open: usize) -> bool {
    if open == 0 || !matches!(tokens[open - 1].kind, TokenKind::Ident(_)) {
        return false;
    }
    nearest_unclosed(tokens, open, TokenKind::LBrace, TokenKind::RBrace)
        .is_some_and(|brace| is_named_declaration_body(tokens, brace, "enum"))
}

fn is_function_parameter_list(tokens: &[&Token], open: usize) -> bool {
    let Some(name) = open.checked_sub(1) else {
        return false;
    };
    if !matches!(tokens[name].kind, TokenKind::Ident(_)) {
        return false;
    }
    if name > 0 && matches!(&tokens[name - 1].kind, TokenKind::Ident(keyword) if keyword == "fn") {
        return true;
    }
    name >= 3
        && matches!(tokens[name - 1].kind, TokenKind::Dot)
        && matches!(tokens[name - 2].kind, TokenKind::Ident(_))
        && matches!(&tokens[name - 3].kind, TokenKind::Ident(keyword) if keyword == "fn")
}

fn is_named_declaration_body(tokens: &[&Token], open: usize, keyword: &str) -> bool {
    open >= 2
        && matches!(tokens[open - 1].kind, TokenKind::Ident(_))
        && matches!(&tokens[open - 2].kind, TokenKind::Ident(name) if name == keyword)
}

fn is_state_body(tokens: &[&Token], open: usize) -> bool {
    for token in tokens[..open].iter().rev() {
        match &token.kind {
            TokenKind::Ident(name) if name == "state" => return true,
            TokenKind::LBrace | TokenKind::RBrace => return false,
            _ => {}
        }
    }
    false
}

fn nearest_unclosed(
    tokens: &[&Token],
    before: usize,
    open: TokenKind,
    close: TokenKind,
) -> Option<usize> {
    let mut depth = 0_u32;
    for index in (0..before).rev() {
        if tokens[index].kind == close {
            depth += 1;
        } else if tokens[index].kind == open {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}

fn matching_open(
    tokens: &[&Token],
    close_index: usize,
    open: TokenKind,
    close: TokenKind,
) -> Option<usize> {
    let mut depth = 0_u32;
    for index in (0..close_index).rev() {
        if tokens[index].kind == close {
            depth += 1;
        } else if tokens[index].kind == open {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}
