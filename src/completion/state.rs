//! Contextual completion for state declarations and named memory layouts.

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, identifier_span, lexer,
    types::add_type_completions,
};
use crate::{
    ast::{Program, Span},
    lexer::TokenKind,
    stdlib::StandardLibrary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    State,
    NamedLayout,
    AttachmentLayout,
    ConditionalState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateBodyKind {
    Unknown,
    Fields,
    Layouts,
}

pub(super) fn complete_state_dsl(
    source: &str,
    syntax: &Program,
    offset: usize,
    standard_library: &StandardLibrary,
) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let lexed = lexer::lex_lossless(source).ok()?;
    let tokens = lexed.tokens().collect::<Vec<_>>();
    let state_open = state_open_containing(&tokens, source.len(), offset)?;
    let (context_open, context) = innermost_context(&tokens, state_open, offset)?;
    let provider_is_specialized = syntax
        .state
        .as_ref()
        .is_some_and(|state| state.provider.is_some());
    let body_kind = if context != Context::State {
        StateBodyKind::Fields
    } else {
        state_body_kind(&tokens, state_open, offset)
    };
    let delimiter = if body_kind == StateBodyKind::Layouts {
        TokenKind::Comma
    } else {
        TokenKind::Semicolon
    };
    let segment = current_segment(&tokens, context_open, offset, delimiter);
    let significant = segment
        .iter()
        .copied()
        .filter(|token| !matches!(token.kind, TokenKind::DocComment(_)))
        .collect::<Vec<_>>();
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);

    if significant.is_empty()
        || significant.len() == 1
            && significant[0].span == replacement
            && matches!(significant[0].kind, TokenKind::Ident(_))
    {
        if context == Context::AttachmentLayout {
            add_dimension_completion(&mut builder);
            return Some(builder.finish());
        }
        if body_kind != StateBodyKind::Layouts {
            add_field_completions(&mut builder, provider_is_specialized);
        }
        if context == Context::State && body_kind != StateBodyKind::Fields {
            add_layout_completion(&mut builder, body_kind == StateBodyKind::Unknown);
        }
        return Some(builder.finish());
    }

    if context == Context::State && body_kind == StateBodyKind::Layouts {
        return Some(builder.finish());
    }

    if significant
        .iter()
        .any(|token| matches!(token.kind, TokenKind::Assign))
    {
        // The remainder is an ordinary expression and should retain normal
        // expression, member, and lexical completion.
        return None;
    }

    if let Some(at) = significant
        .iter()
        .position(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "at"))
    {
        return complete_pointer_tail(
            source,
            offset,
            replacement,
            &significant[at + 1..],
            provider_is_specialized,
        );
    }

    let colon = significant
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Colon));
    if let Some(colon) = colon {
        let after_colon = &significant[colon + 1..];
        if after_colon.is_empty()
            || after_colon.len() == 1
                && after_colon[0].span == replacement
                && replacement.start < offset
                && matches!(after_colon[0].kind, TokenKind::Ident(_))
        {
            add_type_completions(&mut builder, syntax, standard_library);
            return Some(builder.finish());
        }
        if replacement.start == offset
            || after_colon
                .last()
                .is_some_and(|token| token.span == replacement)
        {
            add_field_source_completions(&mut builder);
            return Some(builder.finish());
        }
        return Some(builder.finish());
    }

    if (significant.len() == 1 && replacement.start == offset
        || significant.len() == 2 && significant[1].span == replacement)
        && matches!(significant[0].kind, TokenKind::Ident(_))
    {
        add_field_source_completions(&mut builder);
    }
    Some(builder.finish())
}

fn state_open_containing(
    tokens: &[&crate::lexer::Token],
    source_len: usize,
    offset: usize,
) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        if !matches!(&token.kind, TokenKind::Ident(name) if name == "state") {
            return None;
        }
        let open = tokens[index + 1..]
            .iter()
            .position(|token| matches!(token.kind, TokenKind::LBrace))?
            + index
            + 1;
        let close = super::matching_closing_brace(tokens, open);
        let closing_start = close.map_or(source_len, |close| tokens[close].span.start);
        (tokens[open].span.end <= offset && offset <= closing_start).then_some(open)
    })
}

fn innermost_context(
    tokens: &[&crate::lexer::Token],
    state_open: usize,
    offset: usize,
) -> Option<(usize, Context)> {
    let mut braces = vec![(state_open, Some(Context::State))];
    for (index, token) in tokens.iter().enumerate().skip(state_open + 1) {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => {
                let context = braces
                    .last()
                    .and_then(|(_, context)| *context)
                    .and_then(|context| {
                        (context == Context::State).then(|| {
                            declaration_group_kind(tokens, braces.last().unwrap().0, index)
                        })?
                    });
                braces.push((index, context));
            }
            TokenKind::RBrace => {
                braces.pop();
            }
            _ => {}
        }
    }
    let (open, context) = braces.last().copied()?;
    Some((open, context?))
}

fn declaration_group_kind(
    tokens: &[&crate::lexer::Token],
    parent_open: usize,
    child_open: usize,
) -> Option<Context> {
    let segment = current_segment(
        tokens,
        parent_open,
        tokens[child_open].span.start,
        TokenKind::Comma,
    );
    let significant = segment
        .into_iter()
        .filter(|token| !matches!(token.kind, TokenKind::DocComment(_)))
        .collect::<Vec<_>>();
    if matches!(significant.first().map(|token| &token.kind), Some(TokenKind::Ident(name)) if name == "if")
    {
        return Some(Context::ConditionalState);
    }
    if !matches!(significant.first().map(|token| &token.kind), Some(TokenKind::Ident(name)) if name == "layout")
    {
        return None;
    }
    Some(if significant.len() == 1 {
        Context::AttachmentLayout
    } else {
        Context::NamedLayout
    })
}

fn state_body_kind(
    tokens: &[&crate::lexer::Token],
    state_open: usize,
    offset: usize,
) -> StateBodyKind {
    let mut brace_depth = 0_u32;
    let mut bracket_depth = 0_u32;
    let mut paren_depth = 0_u32;
    for token in tokens.iter().skip(state_open + 1) {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            TokenKind::LBracket => bracket_depth += 1,
            TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
            TokenKind::LParen => paren_depth += 1,
            TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
            TokenKind::DocComment(_) => {}
            TokenKind::Ident(ref name)
                if brace_depth == 0 && bracket_depth == 0 && paren_depth == 0 =>
            {
                return if name == "layout" {
                    let structured = tokens
                        .iter()
                        .skip_while(|candidate| candidate.span.end <= token.span.end)
                        .find(|candidate| !matches!(candidate.kind, TokenKind::DocComment(_)))
                        .is_some_and(|candidate| matches!(candidate.kind, TokenKind::LBrace));
                    if structured {
                        StateBodyKind::Fields
                    } else {
                        StateBodyKind::Layouts
                    }
                } else {
                    StateBodyKind::Fields
                };
            }
            _ => {}
        }
    }
    StateBodyKind::Unknown
}

fn current_segment<'a>(
    tokens: &'a [&crate::lexer::Token],
    open: usize,
    offset: usize,
    delimiter: TokenKind,
) -> Vec<&'a crate::lexer::Token> {
    let mut brace_depth = 0_u32;
    let mut bracket_depth = 0_u32;
    let mut paren_depth = 0_u32;
    let mut start = open + 1;
    for (relative, token) in tokens[open + 1..].iter().enumerate() {
        let index = open + relative + 1;
        if token.span.start >= offset {
            break;
        }
        let at_top = brace_depth == 0 && bracket_depth == 0 && paren_depth == 0;
        if at_top && token.kind == delimiter {
            start = index + 1;
            continue;
        }
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            TokenKind::LBracket => bracket_depth += 1,
            TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
            TokenKind::LParen => paren_depth += 1,
            TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
    }
    tokens[start..]
        .iter()
        .take_while(|token| token.span.start < offset)
        .copied()
        .collect()
}

fn add_field_completions(builder: &mut CompletionBuilder, provider_is_specialized: bool) {
    add_snippet(
        builder,
        "expression field",
        "state field",
        "${1:name} = ${2:expression};",
        "Adds a state field computed by an ordinary expression on every poll.",
    );
    add_snippet(
        builder,
        "memory field",
        "typed memory state field",
        "${1:name}: ${2:i32} at ${3:0x1000};",
        "Reads a typed value from an address on every poll.",
    );
    add_snippet(
        builder,
        "inferred memory field",
        "inferred memory state field",
        "${1:name} at ${2:0x1000};",
        "Reads a value whose memory type is inferred from its uses.",
    );
    if provider_is_specialized {
        return;
    }
    add_snippet(
        builder,
        "module pointer field",
        "module-relative state field",
        "${1:name}: ${2:i32} at \"${3:game.dll}\", ${4:0x1000};",
        "Reads a typed value through a module-relative pointer path.",
    );
    add_snippet(
        builder,
        "UTF-8 string field",
        "bounded native string state field",
        "${1:name} at \"${2:game.dll}\", ${3:0x1000} as utf8(${4:64});",
        "Reads a bounded, null-terminated UTF-8 string through a pointer path.",
    );
    add_snippet(
        builder,
        "UTF-16LE string field",
        "bounded native string state field",
        "${1:name} at \"${2:game.dll}\", ${3:0x1000} as utf16le(${4:64});",
        "Reads a bounded, null-terminated UTF-16LE string through a pointer path.",
    );
}

fn add_layout_completion(builder: &mut CompletionBuilder, allow_dimensions: bool) {
    if allow_dimensions {
        add_snippet(
            builder,
            "layout dimensions",
            "attachment-wide layout dimensions",
            "layout {\n\t${1:dimension}: ${2:Enum},\n}",
            "Declares independent enum-valued facts that describe the selected attachment layout.",
        );
    }
    add_snippet(
        builder,
        "named layout",
        "version-specific state layout",
        "layout ${1:Name} {\n\t$0\n},",
        "Adds one named memory layout. An `onAttach` block selects its generated `StateLayout` variant.",
    );
}

fn add_dimension_completion(builder: &mut CompletionBuilder) {
    add_snippet(
        builder,
        "layout dimension",
        "enum-valued layout dimension",
        "${1:name}: ${2:Enum},",
        "Adds one independent enum-valued fact to the attachment-wide `Layout` record.",
    );
}

fn add_field_source_completions(builder: &mut CompletionBuilder) {
    add_snippet(
        builder,
        "at",
        "memory state source",
        "at ${1:0x1000};",
        "Reads this field from an address or pointer path.",
    );
    add_snippet(
        builder,
        "=",
        "expression state source",
        "= ${1:expression};",
        "Computes this field with an ordinary expression on every poll.",
    );
}

fn complete_pointer_tail(
    source: &str,
    offset: usize,
    replacement: Span,
    tail: &[&crate::lexer::Token],
    provider_is_specialized: bool,
) -> Option<CompletionList> {
    if tail
        .iter()
        .any(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "if"))
    {
        return None;
    }
    if let Some(as_index) = tail
        .iter()
        .position(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "as"))
    {
        // `complete_state_decoder` owns the decoder name itself. Once the
        // decoder call is complete, this resumes with the optional filter.
        let decoder_end = tail[as_index + 1..]
            .iter()
            .rposition(|token| matches!(token.kind, TokenKind::RParen))
            .map(|index| as_index + 1 + index)?;
        let remaining = &tail[decoder_end + 1..];
        if !remaining.is_empty() && !(remaining.len() == 1 && remaining[0].span == replacement) {
            return None;
        }
        let mut builder =
            CompletionBuilder::new(source[replacement.start..offset].to_owned(), replacement);
        add_filter_completion(&mut builder);
        return Some(builder.finish());
    }
    let has_address = tail
        .iter()
        .any(|token| matches!(token.kind, TokenKind::Int(_)));
    let expects_offset = tail
        .last()
        .is_some_and(|token| matches!(token.kind, TokenKind::Comma));
    if !has_address || expects_offset {
        return Some(
            CompletionBuilder::new(source[replacement.start..offset].to_owned(), replacement)
                .finish(),
        );
    }

    let mut builder =
        CompletionBuilder::new(source[replacement.start..offset].to_owned(), replacement);
    if !provider_is_specialized {
        add_snippet(
            &mut builder,
            "as",
            "native string decoder",
            "as ${1|utf8,utf16le|}(${2:maximum})",
            "Decodes the pointer target as a bounded native string.",
        );
    }
    add_filter_completion(&mut builder);
    Some(builder.finish())
}

fn add_filter_completion(builder: &mut CompletionBuilder) {
    add_snippet(
        builder,
        "if",
        "transactional state-field filter",
        "if ${1:condition} { Err(${2:\"rejected state value\"}) } else { value }",
        "Accepts the candidate `value` or returns an error to retain the previously accepted field value.",
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
