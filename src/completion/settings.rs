//! Contextual completion for the declarative settings grammar.

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, identifier_span, lexer,
};
use crate::{ast::Span, lexer::TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    Entries,
    ChoiceOptions,
    FileFilters,
    FamilyEntry,
}

pub(super) fn complete_settings_dsl(source: &str, offset: usize) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let lexed = lexer::lex_lossless(source).ok()?;
    let tokens = lexed.tokens().collect::<Vec<_>>();
    let mut braces = Vec::<(usize, Option<Context>)>::new();
    let mut previous: Option<usize> = None;

    for (index, token) in tokens.iter().enumerate() {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => {
                let parent = braces.last().and_then(|(_, context)| *context);
                let context = match previous.map(|previous| &tokens[previous].kind) {
                    Some(TokenKind::Ident(name)) if name == "settings" => Some(Context::Entries),
                    Some(TokenKind::Ident(name)) if name == "choice" => {
                        Some(Context::ChoiceOptions)
                    }
                    Some(TokenKind::Ident(name)) if name == "file" => Some(Context::FileFilters),
                    Some(TokenKind::String(_)) if parent == Some(Context::Entries) => {
                        Some(Context::Entries)
                    }
                    _ if parent == Some(Context::Entries)
                        && segment_starts_with_for(&tokens, braces.last().unwrap().0, index) =>
                    {
                        Some(Context::FamilyEntry)
                    }
                    _ => None,
                };
                braces.push((index, context));
            }
            TokenKind::RBrace => {
                braces.pop();
            }
            _ => {}
        }
        previous = Some(index);
    }

    let (open, context) = braces.last().copied()?;
    let context = context?;
    let segment = current_segment(&tokens, open, offset);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);

    match context {
        Context::Entries => {
            if segment
                .iter()
                .any(|token| matches!(token.kind, TokenKind::Colon))
            {
                add_kind_completions(&mut builder);
            } else if segment_is_empty_or_prefix(&segment, replacement) {
                add_entry_completions(&mut builder);
            }
        }
        Context::ChoiceOptions if segment_is_empty_or_prefix(&segment, replacement) => {
            add_snippet(
                &mut builder,
                "choice option",
                "choice option",
                "\"${1:Label}\" => ${2:Enum}.${3:Variant}${4: default},",
                "Adds one enum-backed choice option. Exactly one option may be marked `default`.",
            );
        }
        Context::FileFilters if segment_is_empty_or_prefix(&segment, replacement) => {
            add_snippet(
                &mut builder,
                "named filter",
                "file-name filter",
                "\"${1:Files}\" => \"${2:*.ext}\",",
                "Adds a labeled file-name glob filter.",
            );
            add_snippet(
                &mut builder,
                "fallback filter",
                "file-name filter",
                "_ => \"${1:*.*}\",",
                "Adds an unlabeled fallback file-name glob filter.",
            );
            add_snippet(
                &mut builder,
                "MIME filter",
                "MIME filter",
                "mime => \"${1:type/*}\",",
                "Adds a MIME-type filter.",
            );
        }
        Context::ChoiceOptions | Context::FileFilters | Context::FamilyEntry => {}
    }
    Some(builder.finish())
}

fn segment_starts_with_for(
    tokens: &[&crate::lexer::Token],
    parent_open: usize,
    child_open: usize,
) -> bool {
    current_segment(tokens, parent_open, tokens[child_open].span.start)
        .first()
        .is_some_and(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "for"))
}

fn current_segment<'a>(
    tokens: &'a [&crate::lexer::Token],
    open: usize,
    offset: usize,
) -> Vec<&'a crate::lexer::Token> {
    let mut depth = 1_u32;
    let mut start = open + 1;
    for (relative, token) in tokens[open + 1..].iter().enumerate() {
        let index = open + relative + 1;
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Comma if depth == 1 => start = index + 1,
            _ => {}
        }
    }
    tokens[start..]
        .iter()
        .take_while(|token| token.span.start < offset)
        .copied()
        .collect()
}

fn segment_is_empty_or_prefix(segment: &[&crate::lexer::Token], replacement: Span) -> bool {
    segment.iter().all(|token| {
        matches!(token.kind, TokenKind::DocComment(_))
            || matches!(token.kind, TokenKind::Ident(_)) && token.span == replacement
    })
}

fn add_entry_completions(builder: &mut CompletionBuilder) {
    add_snippet(
        builder,
        "boolean setting",
        "settings declaration",
        "\"${1:Label}\" => ${2:name}: ${3|true,false|},",
        "Adds a boolean setting whose value is available through `settings.name`.",
    );
    add_snippet(
        builder,
        "settings group",
        "settings heading",
        "\"${1:Group}\" {\n\t$0\n},",
        "Adds a visual settings heading. Groups may be nested.",
    );
    add_snippet(
        builder,
        "choice setting",
        "settings declaration",
        "\"${1:Label}\" => ${2:name}: choice {\n\t\"${3:Option}\" => ${4:Enum}.${5:Variant} default,\n},",
        "Adds an enum-backed choice setting.",
    );
    add_snippet(
        builder,
        "file setting",
        "settings declaration",
        "\"${1:Label}\" => ${2:name}: file {\n\t\"${3:Files}\" => \"${4:*.*}\",\n},",
        "Adds a file-selector setting with a file-name filter.",
    );
    add_snippet(
        builder,
        "for setting family",
        "generated boolean settings",
        "for ${1:item} in ${2:0}..=${3:10} {\n\t`${4:Item} {${1:item}}` key `{${1:item}}`: ${5|true,false|},\n},",
        "Generates a finite family of boolean settings at compile time.",
    );
}

fn add_kind_completions(builder: &mut CompletionBuilder) {
    for (label, documentation) in [
        ("true", "Creates a boolean setting enabled by default."),
        ("false", "Creates a boolean setting disabled by default."),
    ] {
        add_snippet(builder, label, "boolean default", label, documentation);
    }
    add_snippet(
        builder,
        "choice",
        "choice setting",
        "choice {\n\t\"${1:Option}\" => ${2:Enum}.${3:Variant} default,\n},",
        "Creates an enum-backed choice setting.",
    );
    add_snippet(
        builder,
        "file",
        "file setting",
        "file {\n\t\"${1:Files}\" => \"${2:*.*}\",\n},",
        "Creates a file-selector setting.",
    );
}

fn add_snippet(
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
        documentation_uri: None,
        insert_text: insert_text.to_owned(),
        is_snippet: true,
    });
}
