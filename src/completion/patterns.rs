//! Type-directed completion for match patterns.
//!
//! Pattern completion deliberately lives beside the other compiler-owned
//! completion grammars. Editor frontends receive already-filtered candidates;
//! they do not need to duplicate the pattern grammar or semantic type model.

use std::collections::BTreeSet;

use crate::{
    ast::{Expr, ExprKind, StructId},
    database::{CompilerDatabase, SemanticQueryResult, SemanticSnapshot},
    documentation::symbol_uri,
    lexer::{Token, TokenKind},
    stdlib::{
        CoreTypeId, StandardLibrary, StdlibCapabilityId, StdlibSymbolId, StdlibTypeConstructorId,
        StdlibTypeId,
    },
    type_display::display_type,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, CompletionRequest,
    matching_closing_brace, render_documentation,
};

pub(super) fn complete_pattern(
    database: &mut CompilerDatabase,
    request: &CompletionRequest<'_>,
    library: &StandardLibrary,
) -> SemanticQueryResult<Option<CompletionList>> {
    let snapshot = database.semantic_snapshot()?;
    let Some((open, value)) = enclosing_match(
        snapshot.syntax(),
        &request.tokens,
        request.replacement.start,
    ) else {
        return Ok(None);
    };
    let Some(segment) = current_pattern_segment(&request.tokens, open, request.replacement.start)
    else {
        return Ok(None);
    };
    let Some(expected) = snapshot.semantics().expression_type(value.id) else {
        return Ok(None);
    };
    let Some(site) = analyze_pattern_prefix(segment, expected, &snapshot) else {
        return Ok(None);
    };

    let prefix = request.source[request.replacement.start..request.offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, request.replacement);
    match site {
        PatternSite::Value {
            expected,
            qualified_enum,
        } => add_value_patterns(&mut builder, expected, qualified_enum, &snapshot, library),
        PatternSite::StructFields { structure, used } => {
            add_struct_fields(&mut builder, structure, &used, &snapshot)
        }
    }
    Ok(Some(builder.finish()))
}

#[derive(Debug)]
enum PatternSite {
    Value {
        expected: TypeId,
        qualified_enum: bool,
    },
    StructFields {
        structure: StructId,
        used: BTreeSet<String>,
    },
}

fn enclosing_match<'ast>(
    syntax: &'ast crate::ast::Program,
    tokens: &[&Token],
    offset: usize,
) -> Option<(usize, &'ast Expr)> {
    struct Finder<'ast, 'tokens> {
        tokens: &'tokens [&'tokens Token],
        offset: usize,
        found: Option<(usize, &'ast Expr)>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast, '_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Match { value, .. } = &expression.kind
                && let Some(open) = self.tokens.iter().position(|token| {
                    token.span.start >= value.span.end && token.kind == TokenKind::LBrace
                })
            {
                let close = matching_closing_brace(self.tokens, open)
                    .map_or(usize::MAX, |close| self.tokens[close].span.end);
                if self.tokens[open].span.end <= self.offset
                    && self.offset <= close
                    && self.found.as_ref().is_none_or(|(previous, _)| {
                        self.tokens[*previous].span.start < self.tokens[open].span.start
                    })
                {
                    self.found = Some((open, value));
                }
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        tokens,
        offset,
        found: None,
    };
    finder.visit_program(syntax);
    finder.found
}

/// Returns only the tokens belonging to the current arm's pattern. A top-level
/// `=>` or guard switches back to ordinary expression completion, while commas
/// nested in struct and array patterns remain part of this segment.
fn current_pattern_segment<'a>(
    tokens: &'a [&'a Token],
    open: usize,
    replacement_start: usize,
) -> Option<&'a [&'a Token]> {
    let cursor = tokens
        .iter()
        .position(|token| token.span.start >= replacement_start)
        .unwrap_or(tokens.len());
    let mut start = open + 1;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut in_value = false;

    for (index, token) in tokens.iter().enumerate().take(cursor).skip(open + 1) {
        let top_level = parentheses == 0 && brackets == 0 && braces == 0;
        if top_level {
            match &token.kind {
                TokenKind::Comma => {
                    start = index + 1;
                    in_value = false;
                    continue;
                }
                TokenKind::FatArrow => in_value = true,
                TokenKind::Ident(name) if name == "if" && !in_value => return None,
                _ => {}
            }
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }

    (!in_value).then_some(&tokens[start..cursor])
}

fn update_depths(
    token: &TokenKind,
    parentheses: &mut usize,
    brackets: &mut usize,
    braces: &mut usize,
) {
    match token {
        TokenKind::LParen => *parentheses += 1,
        TokenKind::RParen => *parentheses = parentheses.saturating_sub(1),
        TokenKind::LBracket => *brackets += 1,
        TokenKind::RBracket => *brackets = brackets.saturating_sub(1),
        TokenKind::LBrace => *braces += 1,
        TokenKind::RBrace => *braces = braces.saturating_sub(1),
        _ => {}
    }
}

fn analyze_pattern_prefix(
    tokens: &[&Token],
    expected: TypeId,
    snapshot: &SemanticSnapshot,
) -> Option<PatternSite> {
    let tokens = after_last_top_level(tokens, TokenKind::Or);
    if tokens.is_empty() {
        return Some(PatternSite::Value {
            expected,
            qualified_enum: false,
        });
    }

    let unmatched = unmatched_openings(tokens);
    let Some(&open) = unmatched.first() else {
        let qualified_enum = matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Dot))
            && enum_qualifier_matches(&tokens[..tokens.len() - 1], expected, snapshot);
        return qualified_enum.then_some(PatternSite::Value {
            expected,
            qualified_enum: true,
        });
    };

    match tokens[open].kind {
        TokenKind::LParen => {
            let payload = constructor_payload_type(&tokens[..open], expected, snapshot)?;
            analyze_pattern_prefix(&tokens[open + 1..], payload, snapshot)
        }
        TokenKind::LBracket => {
            let TypeKind::Array { element, .. } = snapshot.semantics().types().kind(expected)
            else {
                return None;
            };
            let tail = after_last_top_level(&tokens[open + 1..], TokenKind::Comma);
            analyze_pattern_prefix(tail, *element, snapshot)
        }
        TokenKind::LBrace => {
            let TypeKind::Struct(structure) = snapshot.semantics().types().kind(expected) else {
                return None;
            };
            let declaration = snapshot
                .syntax()
                .structs
                .iter()
                .find(|candidate| candidate.id == *structure)?;
            let Some(TokenKind::Ident(name)) = tokens[..open].last().map(|token| &token.kind)
            else {
                return None;
            };
            if name.as_str() != declaration.name.as_str() {
                return None;
            }

            let contents = &tokens[open + 1..];
            let chunks = top_level_chunks(contents, TokenKind::Comma);
            let mut used = BTreeSet::new();
            for chunk in chunks.iter().take(chunks.len().saturating_sub(1)) {
                if let Some(TokenKind::Ident(name)) = chunk.first().map(|token| &token.kind) {
                    used.insert(name.clone());
                }
            }
            let tail = chunks.last().copied().unwrap_or_default();
            if let Some(colon) = top_level_token(tail, TokenKind::Colon) {
                let name = tail[..colon].iter().find_map(|token| match &token.kind {
                    TokenKind::Ident(name) => Some(name.as_str()),
                    _ => None,
                })?;
                let field = declaration.fields.iter().find(|field| field.name == name)?;
                let field_type = snapshot.semantics().struct_field_type(field.id)?;
                analyze_pattern_prefix(&tail[colon + 1..], field_type, snapshot)
            } else {
                if let Some(TokenKind::Ident(name)) = tail.first().map(|token| &token.kind) {
                    used.insert(name.clone());
                }
                Some(PatternSite::StructFields {
                    structure: *structure,
                    used,
                })
            }
        }
        _ => None,
    }
}

fn unmatched_openings(tokens: &[&Token]) -> Vec<usize> {
    let mut stack: Vec<(TokenKind, usize)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                stack.push((token.kind.clone(), index));
            }
            TokenKind::RParen => close_delimiter(&mut stack, TokenKind::LParen),
            TokenKind::RBracket => close_delimiter(&mut stack, TokenKind::LBracket),
            TokenKind::RBrace => close_delimiter(&mut stack, TokenKind::LBrace),
            _ => {}
        }
    }
    stack.into_iter().map(|(_, index)| index).collect()
}

fn close_delimiter(stack: &mut Vec<(TokenKind, usize)>, opening: TokenKind) {
    if stack.last().is_some_and(|(kind, _)| *kind == opening) {
        stack.pop();
    }
}

fn after_last_top_level<'a>(tokens: &'a [&'a Token], separator: TokenKind) -> &'a [&'a Token] {
    top_level_chunks(tokens, separator)
        .last()
        .copied()
        .unwrap_or_default()
}

fn top_level_chunks<'a>(tokens: &'a [&'a Token], separator: TokenKind) -> Vec<&'a [&'a Token]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if parentheses == 0 && brackets == 0 && braces == 0 && token.kind == separator {
            chunks.push(&tokens[start..index]);
            start = index + 1;
            continue;
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }
    chunks.push(&tokens[start..]);
    chunks
}

fn top_level_token(tokens: &[&Token], searched: TokenKind) -> Option<usize> {
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if parentheses == 0 && brackets == 0 && braces == 0 && token.kind == searched {
            return Some(index);
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }
    None
}

fn constructor_payload_type(
    head: &[&Token],
    expected: TypeId,
    snapshot: &SemanticSnapshot,
) -> Option<TypeId> {
    let name = head.iter().rev().find_map(|token| match &token.kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    })?;
    match (name, snapshot.semantics().types().kind(expected)) {
        ("Some", TypeKind::Option { value, .. }) | ("Ok", TypeKind::Result { value, .. }) => {
            Some(*value)
        }
        ("Err", TypeKind::Result { .. }) => Some(
            snapshot
                .semantics()
                .types()
                .id_for_standard(StdlibTypeId::String),
        ),
        (
            "Item",
            TypeKind::Application {
                constructor,
                arguments,
                ..
            },
        ) if *constructor == StdlibTypeConstructorId::IteratorStep => arguments.first().copied(),
        (variant, TypeKind::Enum(enumeration)) => {
            let declaration = snapshot
                .enum_types()
                .iter()
                .find(|candidate| candidate.id == *enumeration)?;
            let variant = declaration
                .variants
                .iter()
                .find(|candidate| candidate.name == variant)?;
            snapshot.semantics().enum_variant_payload(variant.id)
        }
        _ => None,
    }
}

fn enum_qualifier_matches(head: &[&Token], expected: TypeId, snapshot: &SemanticSnapshot) -> bool {
    let Some(TokenKind::Ident(name)) = head.last().map(|token| &token.kind) else {
        return false;
    };
    match snapshot.semantics().types().kind(expected) {
        TypeKind::Enum(enumeration) => snapshot
            .enum_types()
            .iter()
            .any(|candidate| candidate.id == *enumeration && candidate.name == *name),
        TypeKind::Standard(standard) => {
            snapshot
                .context()
                .standard_library()
                .type_decl(*standard)
                .name
                == name
        }
        _ => false,
    }
}

fn add_value_patterns(
    builder: &mut CompletionBuilder,
    expected: TypeId,
    qualified_enum: bool,
    snapshot: &SemanticSnapshot,
    library: &StandardLibrary,
) {
    let detail = format!("pattern for `{}`", display_type(expected, snapshot));
    if !qualified_enum {
        builder.add(pattern_item("_", "_", false, &detail, None, None));
    }
    match snapshot.semantics().types().kind(expected) {
        TypeKind::Builtin(CoreTypeId::Bool) => {
            add_plain(builder, "false", &detail);
            add_plain(builder, "true", &detail);
        }
        TypeKind::Builtin(CoreTypeId::Char) => builder.add(pattern_item(
            "character literal",
            "'${1:x}'",
            true,
            &detail,
            Some("Matches one character value.".to_owned()),
            None,
        )),
        TypeKind::Builtin(core)
            if library.core_type_has_capability(*core, StdlibCapabilityId::Integer) =>
        {
            builder.add(pattern_item(
                "integer literal",
                "${1:0}",
                true,
                &detail,
                Some("Matches one integer value, including a negative signed value.".to_owned()),
                None,
            ));
            builder.add(pattern_item(
                "exclusive integer range",
                "${1:0}..<${2:10}",
                true,
                &detail,
                Some(
                    "Matches integers from the lower bound up to, but excluding, the upper bound."
                        .to_owned(),
                ),
                None,
            ));
            builder.add(pattern_item(
                "inclusive integer range",
                "${1:0}..=${2:10}",
                true,
                &detail,
                Some("Matches integers from the lower bound through the upper bound.".to_owned()),
                None,
            ));
        }
        TypeKind::Standard(StdlibTypeId::String) => builder.add(pattern_item(
            "string literal",
            "\"${1:value}\"",
            true,
            &detail,
            Some("Matches one string value.".to_owned()),
            None,
        )),
        TypeKind::Standard(StdlibTypeId::FileVersion) => builder.add(pattern_item(
            "file-version literal",
            "v\"${1:1.0.0.0}\"",
            true,
            &detail,
            Some("Matches one four-component file version.".to_owned()),
            None,
        )),
        TypeKind::Standard(standard) => {
            let owner = library.type_decl(*standard);
            for variant in library.variants_of(*standard) {
                let label = if qualified_enum {
                    variant.name.to_owned()
                } else {
                    format!("{}.{}", owner.name, variant.name)
                };
                builder.add(pattern_item(
                    &label,
                    &label,
                    false,
                    &detail,
                    Some(render_documentation(&variant.documentation)),
                    Some(symbol_uri(StdlibSymbolId::Variant(variant.id), library)),
                ));
            }
        }
        TypeKind::Option { .. } => {
            add_plain(builder, "None", &detail);
            add_snippet(builder, "Some", "Some(${1:_})", &detail);
        }
        TypeKind::Result { .. } => {
            add_snippet(builder, "Err", "Err(${1:_})", &detail);
            add_snippet(builder, "Ok", "Ok(${1:_})", &detail);
        }
        TypeKind::Application { constructor, .. }
            if *constructor == StdlibTypeConstructorId::IteratorStep =>
        {
            add_plain(builder, "End", &detail);
            add_snippet(builder, "Item", "Item(${1:_})", &detail);
        }
        TypeKind::Enum(enumeration) => {
            let Some(declaration) = snapshot
                .enum_types()
                .iter()
                .find(|candidate| candidate.id == *enumeration)
            else {
                return;
            };
            for variant in &declaration.variants {
                let label = if qualified_enum {
                    variant.name.clone()
                } else {
                    format!("{}.{}", declaration.name, variant.name)
                };
                let insert = if variant.payload.is_some() {
                    format!("{label}(${{1:_}})")
                } else {
                    label.clone()
                };
                builder.add(pattern_item(
                    &label,
                    &insert,
                    variant.payload.is_some(),
                    &detail,
                    variant.documentation.clone(),
                    None,
                ));
            }
        }
        TypeKind::Struct(structure) => {
            let Some(declaration) = snapshot
                .syntax()
                .structs
                .iter()
                .find(|candidate| candidate.id == *structure)
            else {
                return;
            };
            let insert = if declaration.fields.is_empty() {
                format!("{} {{}}", declaration.name)
            } else {
                let fields = declaration
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| format!("{}: ${{{}:_}}", field.name, index + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {fields} }}", declaration.name)
            };
            builder.add(pattern_item(
                &declaration.name,
                &insert,
                !declaration.fields.is_empty(),
                &detail,
                declaration.documentation.clone(),
                None,
            ));
        }
        TypeKind::Array {
            length: Some(length),
            ..
        } if *length <= 32 => {
            let elements = (0..*length)
                .map(|index| format!("${{{}:_}}", index + 1))
                .collect::<Vec<_>>()
                .join(", ");
            builder.add(pattern_item(
                "fixed-array pattern",
                &format!("[{elements}]"),
                *length != 0,
                &detail,
                Some(format!("Matches all {length} elements of the fixed array.")),
                None,
            ));
        }
        _ => {}
    }
}

fn add_struct_fields(
    builder: &mut CompletionBuilder,
    structure: StructId,
    used: &BTreeSet<String>,
    snapshot: &SemanticSnapshot,
) {
    let Some(declaration) = snapshot
        .syntax()
        .structs
        .iter()
        .find(|candidate| candidate.id == structure)
    else {
        return;
    };
    for field in &declaration.fields {
        if used.contains(&field.name) {
            continue;
        }
        let detail = snapshot
            .semantics()
            .struct_field_type(field.id)
            .map(|ty| format!("{}: {}", field.name, display_type(ty, snapshot)))
            .unwrap_or_else(|| format!("{} field", declaration.name));
        builder.add(pattern_item(
            &field.name,
            &format!("{}: ${{1:_}}", field.name),
            true,
            &detail,
            field.documentation.clone(),
            None,
        ));
    }
}

fn add_plain(builder: &mut CompletionBuilder, label: &str, detail: &str) {
    builder.add(pattern_item(label, label, false, detail, None, None));
}

fn add_snippet(builder: &mut CompletionBuilder, label: &str, insert: &str, detail: &str) {
    builder.add(pattern_item(label, insert, true, detail, None, None));
}

fn pattern_item(
    label: &str,
    insert_text: &str,
    is_snippet: bool,
    detail: &str,
    documentation: Option<String>,
    documentation_uri: Option<String>,
) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind: if is_snippet {
            CompletionKind::Snippet
        } else if label == "_" {
            CompletionKind::Variable
        } else {
            CompletionKind::EnumMember
        },
        detail: Some(detail.to_owned()),
        documentation,
        documentation_uri,
        insert_text: insert_text.to_owned(),
        is_snippet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completions(source: &str) -> CompletionList {
        let offset = source.find("<|>").expect("completion marker exists");
        let source = source.replacen("<|>", "", 1);
        CompilerDatabase::new(source)
            .completions(offset)
            .expect("completion succeeds")
    }

    fn labels(source: &str) -> Vec<String> {
        completions(source)
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn root_pattern_completion_is_type_directed_and_suppresses_expressions() {
        let source = r#"
state "game.exe" {}
fn inspect(value: bool) {
    return match value {
        <|>true => 1,
        false => 0,
    }
}
"#;
        let labels = labels(source);
        for expected in ["_", "false", "true"] {
            assert!(labels.contains(&expected.to_owned()), "{labels:#?}");
        }
        for expression in ["print", "return", "String", "while"] {
            assert!(!labels.contains(&expression.to_owned()), "{labels:#?}");
        }
    }

    #[test]
    fn integer_pattern_completion_offers_both_explicit_range_shapes() {
        let items = completions(
            r#"
state "game.exe" {}
fn inspect(value: i16) {
    return match value {
        <|>
    }
}
"#,
        )
        .items;
        for (label, insert) in [
            ("exclusive integer range", "${1:0}..<${2:10}"),
            ("inclusive integer range", "${1:0}..=${2:10}"),
        ] {
            let item = items
                .iter()
                .find(|item| item.label == label)
                .unwrap_or_else(|| panic!("missing `{label}` in {items:#?}"));
            assert_eq!(item.insert_text, insert);
            assert!(item.is_snippet);
        }
    }

    #[test]
    fn completion_survives_an_empty_recovered_match_arm() {
        let source = r#"
state "game.exe" {}
fn inspect(value: bool) {
    return match value {
        <|>
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"print".to_owned()));
    }

    #[test]
    fn enum_completion_supports_qualified_and_payload_variants() {
        let source = r#"
enum Mode { Idle, Active(bool) }
state "game.exe" {}
fn inspect(mode: Mode) {
    return match mode {
        <|>Mode.Idle => false,
        Mode.Active(value) => value,
    }
}
"#;
        let completion = completions(source);
        let active = completion
            .items
            .iter()
            .find(|item| item.label == "Mode.Active")
            .expect("payload variant is completed");
        assert_eq!(active.insert_text, "Mode.Active(${1:_})");
        assert!(active.is_snippet);

        let qualified = labels(&source.replace("<|>Mode.Idle", "Mode.<|>Idle"));
        assert!(qualified.contains(&"Idle".to_owned()), "{qualified:#?}");
        assert!(qualified.contains(&"Active".to_owned()), "{qualified:#?}");
        assert!(!qualified.contains(&"Mode.Idle".to_owned()));
    }

    #[test]
    fn standard_enum_completion_uses_the_same_qualification_rules() {
        let source = r#"
state "game.exe" {}
fn inspect(value: TimerState) {
    return match value {
        <|>TimerState.Running => true,
        _ => false,
    }
}
"#;
        let root_labels = labels(source);
        assert!(
            root_labels.contains(&"TimerState.Running".to_owned()),
            "{root_labels:#?}"
        );
        assert!(!root_labels.contains(&"Running".to_owned()));

        let qualified = labels(&source.replace("<|>TimerState.Running", "TimerState.<|>Running"));
        assert!(qualified.contains(&"Running".to_owned()), "{qualified:#?}");
        assert!(!qualified.contains(&"TimerState.Running".to_owned()));
    }

    #[test]
    fn wrapper_and_array_payloads_complete_recursively() {
        let option = r#"
state "game.exe" {}
fn inspect(value: bool?) {
    return match value {
        Some(<|>_) => true,
        None => false,
    }
}
"#;
        let option_labels = labels(option);
        assert!(
            option_labels.contains(&"true".to_owned()),
            "{option_labels:#?}"
        );
        assert!(
            option_labels.contains(&"false".to_owned()),
            "{option_labels:#?}"
        );
        assert!(!option_labels.contains(&"Some".to_owned()));

        let array = r#"
state "game.exe" {}
fn inspect(value: [bool; 2]) {
    return match value {
        [true, <|>_] => true,
        _ => false,
    }
}
"#;
        let labels = labels(array);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"fixed-array pattern".to_owned()));
    }

    #[test]
    fn result_and_iterator_step_completion_offer_only_their_constructors() {
        let result = r#"
state "game.exe" {}
fn inspect(value: bool!) {
    return match value {
        <|>Ok(_) => true,
        Err(_) => false,
    }
}
"#;
        let result_labels = labels(result);
        for expected in ["_", "Err", "Ok"] {
            assert!(
                result_labels.contains(&expected.to_owned()),
                "{result_labels:#?}"
            );
        }
        assert!(!result_labels.contains(&"Some".to_owned()));

        let step = r#"
state "game.exe" {}
fn inspect(value: IteratorStep<bool>) {
    return match value {
        <|>Item(_) => true,
        End => false,
    }
}
"#;
        let labels = labels(step);
        for expected in ["_", "End", "Item"] {
            assert!(labels.contains(&expected.to_owned()), "{labels:#?}");
        }
        assert!(!labels.contains(&"None".to_owned()));
    }

    #[test]
    fn completion_survives_an_unclosed_wrapper_pattern() {
        let source = r#"
state "game.exe" {}
fn inspect(value: bool?) {
    return match value {
        Some(<|>
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"Some".to_owned()));
    }

    #[test]
    fn struct_patterns_complete_only_remaining_fields_and_field_patterns() {
        let source = r#"
struct Position { visible: bool, grounded: bool }
state "game.exe" {}
fn inspect(position: Position) {
    return match position {
        Position { visible: _, <|>grounded: _ } => true,
        _ => false,
    }
}
"#;
        let completion = completions(source);
        assert_eq!(
            completion
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["grounded"]
        );
        assert_eq!(completion.items[0].insert_text, "grounded: ${1:_}");

        let field = source.replace("<|>grounded: _", "grounded: <|>_");
        let labels = labels(&field);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
    }

    #[test]
    fn fixed_array_completion_provides_the_exact_shape() {
        let source = r#"
state "game.exe" {}
fn inspect(value: [bool; 2]) {
    return match value {
        <|>_ => true,
    }
}
"#;
        let completion = completions(source);
        let array = completion
            .items
            .iter()
            .find(|item| item.label == "fixed-array pattern")
            .expect("fixed array shape is completed");
        assert_eq!(array.insert_text, "[${1:_}, ${2:_}]");
        assert!(array.is_snippet);
    }

    #[test]
    fn match_arm_values_keep_ordinary_expression_completion() {
        let source = r#"
state "game.exe" {}
fn inspect(value: bool) {
    return match value {
        true => <|>false,
        false => false,
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"print".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"fixed-array pattern".to_owned()));
    }
}
