//! Completion for the declaration-only module grammar and state headers.

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, catalog_language_completion,
    identifier_span, lexer, render_documentation,
};
use crate::{
    ast::{ActionKind, Program},
    language::{LanguageCatalog, LanguageItemId},
    lexer::TokenKind,
    stdlib::{StandardLibrary, StateProviderProcesses},
};

pub(super) fn complete_top_level(source: &str, syntax: &Program, offset: usize) -> CompletionList {
    let replacement = identifier_span(source, offset);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    let language = LanguageCatalog::new();

    if syntax.state.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::State),
            "state \"${1:game.exe}\" {\n\t$0\n}",
        );
    }
    if syntax.tick_rate.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::TickRate),
            "tickRate {\n\tattached: ${1:120},\n\tdetached: ${2:1},\n}",
        );
    }
    if syntax.settings_span.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::Settings),
            "settings {\n\t$0\n}",
        );
    }

    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Function),
        "fn ${1:name}(${2}) {\n\t$0\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Record),
        "record ${1:Name} {\n\t${2:field}: ${3:Type},\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Enum),
        "enum ${1:Name} {\n\t${2:Variant},\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Let),
        "let ${1:name} = ${2:value}",
    );

    add_plain_snippet(
        &mut builder,
        "debug fn",
        "development-only function",
        "debug fn ${1:name}(${2}) {\n\t$0\n}",
        "Declares a function that is checked in every build and erased from release output.",
    );
    add_plain_snippet(
        &mut builder,
        "debug let",
        "development-only global",
        "debug let ${1:name} = ${2:value}",
        "Declares a global that is checked in every build and erased from release output.",
    );

    for action in all_actions() {
        if syntax
            .actions
            .iter()
            .any(|existing| existing.kind == action)
        {
            continue;
        }
        let item = language.action(action);
        add_catalog_snippet(&mut builder, item, &format!("{} {{\n\t$0\n}}", item.name));
    }

    // Named layouts need an onAttach selector. Preserve the more useful
    // generated selector over the generic empty lifecycle snippet.
    if let Some(state) = syntax
        .state
        .as_ref()
        .filter(|state| state.provider.is_none() && !state.layouts.is_empty())
        && !syntax
            .actions
            .iter()
            .any(|action| action.kind == ActionKind::OnAttach)
    {
        builder.add_scoped(super::layout_selector_completion(state));
    }

    builder.finish()
}

pub(super) fn complete_state_header(
    source: &str,
    offset: usize,
    standard_library: &StandardLibrary,
) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let lexed = lexer::lex_lossless(source).ok()?;
    let tokens = lexed.tokens().collect::<Vec<_>>();
    let state = top_level_state_header(&tokens, offset)?;
    let tail = tokens[state + 1..]
        .iter()
        .copied()
        .take_while(|token| token.span.start < offset)
        .collect::<Vec<_>>();
    if tail
        .iter()
        .any(|token| matches!(token.kind, TokenKind::LBrace))
    {
        return None;
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    if tail.is_empty()
        || tail.len() == 1
            && tail[0].span == replacement
            && matches!(tail[0].kind, TokenKind::Ident(_))
    {
        add_plain_snippet(
            &mut builder,
            "\"game.exe\"",
            "process executable",
            "\"${1:game.exe}\" {\n\t$0\n}",
            "Attaches to one executable name and exposes the native `process` value.",
        );
        add_plain_snippet(
            &mut builder,
            "[\"game.exe\", ...]",
            "process executable list",
            "[\"${1:game.exe}\", \"${2:demo.exe}\"] {\n\t$0\n}",
            "Attaches to any executable in the list and exposes the native `process` value.",
        );
        for provider in standard_library
            .state_providers()
            .iter()
            .filter(|provider| matches!(provider.processes, StateProviderProcesses::Declared(_)))
        {
            builder.add(CompletionItem {
                label: provider.name.to_owned(),
                kind: CompletionKind::Snippet,
                detail: Some("standard-library state provider".to_owned()),
                documentation: Some(render_documentation(&provider.documentation)),
                insert_text: format!("{} {{\n\t$0\n}}", provider.name),
                is_snippet: true,
            });
        }
        return Some(builder.finish());
    }

    let complete_target = matches!(tail.as_slice(), [token] if matches!(token.kind, TokenKind::String(_)))
        || is_complete_process_list(&tail)
        || matches!(tail.as_slice(), [token]
            if matches!(&token.kind, TokenKind::Ident(name)
                if standard_library.state_provider_by_name(name).is_some_and(|provider|
                    matches!(provider.processes, StateProviderProcesses::Declared(_)))));
    if complete_target && replacement.start == offset {
        add_plain_snippet(
            &mut builder,
            "{",
            "state body",
            "{\n\t$0\n}",
            "Begins the persistent watched-state declaration.",
        );
    }
    Some(builder.finish())
}

fn top_level_state_header(tokens: &[&crate::lexer::Token], offset: usize) -> Option<usize> {
    let mut brace_depth = 0_u32;
    let mut candidate = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            TokenKind::Ident(ref name) if brace_depth == 0 && name == "state" => {
                candidate = Some(index);
            }
            _ => {}
        }
    }
    candidate
}

fn is_complete_process_list(tokens: &[&crate::lexer::Token]) -> bool {
    if !matches!(
        tokens.first().map(|token| &token.kind),
        Some(TokenKind::LBracket)
    ) || !matches!(
        tokens.last().map(|token| &token.kind),
        Some(TokenKind::RBracket)
    ) {
        return false;
    }
    let mut expects_string = true;
    for token in &tokens[1..tokens.len() - 1] {
        match (&token.kind, expects_string) {
            (TokenKind::String(_), true) => expects_string = false,
            (TokenKind::Comma, false) => expects_string = true,
            _ => return false,
        }
    }
    !expects_string
}

fn all_actions() -> [ActionKind; 10] {
    [
        ActionKind::Setup,
        ActionKind::OnDetach,
        ActionKind::OnAttach,
        ActionKind::OnStateReady,
        ActionKind::WhileAttached,
        ActionKind::Start,
        ActionKind::Split,
        ActionKind::Reset,
        ActionKind::IsLoading,
        ActionKind::GameTime,
    ]
}

fn add_catalog_snippet(
    builder: &mut CompletionBuilder,
    item: &crate::language::LanguageItem,
    insert_text: &str,
) {
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Snippet,
        item,
        insert_text.to_owned(),
        true,
    ));
}

fn add_plain_snippet(
    builder: &mut CompletionBuilder,
    label: &str,
    detail: &str,
    insert_text: &str,
    documentation: &str,
) {
    builder.add(CompletionItem {
        label: label.to_owned(),
        kind: CompletionKind::Snippet,
        detail: Some(detail.to_owned()),
        documentation: Some(documentation.to_owned()),
        insert_text: insert_text.to_owned(),
        is_snippet: true,
    });
}
