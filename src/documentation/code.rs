//! Semantic code fragments for the generated Markdown reference.

use crate::{
    catalog::Example,
    database::{CompilerDatabase, DefinitionTarget},
    highlight::SemanticTokenKind,
    language::LanguageCatalog,
    lexer::{Lexeme, TokenKind, TriviaKind, lex_lossless},
    stdlib::{StandardLibrary, StdlibSymbolId},
};

use super::reference::{core_type_uri, language_item_uri, relative_document_link, symbol_uri};

#[derive(Debug, Clone)]
struct Annotation {
    start: usize,
    end: usize,
    kind: SemanticTokenKind,
    target: Option<String>,
}

pub(super) fn signature(
    source: &str,
    current_uri: &str,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> String {
    let annotations = lexical_annotations(source, current_uri, primary, library);
    render(source, &annotations)
}

pub(super) fn example(example: Example, current_uri: &str, library: &StandardLibrary) -> String {
    let annotations = semantic_example_annotations(example, current_uri, library)
        .unwrap_or_else(|| lexical_annotations(example.source, current_uri, None, library));
    render(example.source, &annotations)
}

fn semantic_example_annotations(
    example: Example,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<Vec<Annotation>> {
    let program = example.validation_program();
    let visible_start = program.find(example.source)?;
    let visible_end = visible_start.checked_add(example.source.len())?;
    let mut database = CompilerDatabase::with_source_name("stdlib-example.split", program);
    let highlights = database.semantic_highlights().ok()?;
    let mut annotations = Vec::new();
    for highlight in highlights.highlights() {
        if highlight.span.start < visible_start || highlight.span.end > visible_end {
            continue;
        }
        let target = is_linkable(highlight.kind)
            .then(|| database.definition_at(highlight.span.start).ok().flatten())
            .flatten()
            .and_then(|target| target_uri(target, current_uri, library));
        annotations.push(Annotation {
            start: highlight.span.start - visible_start,
            end: highlight.span.end - visible_start,
            kind: highlight.kind,
            target,
        });
    }
    Some(annotations)
}

fn lexical_annotations(
    source: &str,
    current_uri: &str,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> Vec<Annotation> {
    let Ok(lexed) = lex_lossless(source) else {
        return Vec::new();
    };
    let lexemes = lexed.into_lexemes();
    let mut annotations = Vec::new();
    for (index, lexeme) in lexemes.iter().enumerate() {
        let (span, kind, target) = match lexeme {
            Lexeme::Trivia(trivia)
                if matches!(
                    trivia.kind,
                    TriviaKind::LineComment | TriviaKind::BlockComment
                ) =>
            {
                (trivia.span, SemanticTokenKind::Comment, None)
            }
            Lexeme::Trivia(_) => continue,
            Lexeme::Token(token) => {
                let previous = previous_token(&lexemes, index);
                let next = next_token(&lexemes, index);
                let Some(kind) = lexical_kind(&token.kind, previous, next, primary, library) else {
                    continue;
                };
                let target = match &token.kind {
                    TokenKind::Ident(name) => {
                        lexical_target(name, previous, next, primary, current_uri, library)
                    }
                    _ => None,
                };
                (token.span, kind, target)
            }
        };
        if span.start != span.end {
            annotations.push(Annotation {
                start: span.start,
                end: span.end,
                kind,
                target,
            });
        }
    }
    annotations
}

fn previous_token(lexemes: &[Lexeme], index: usize) -> Option<&TokenKind> {
    lexemes[..index]
        .iter()
        .rev()
        .find_map(|lexeme| match lexeme {
            Lexeme::Token(token) if token.kind != TokenKind::Eof => Some(&token.kind),
            _ => None,
        })
}

fn next_token(lexemes: &[Lexeme], index: usize) -> Option<&TokenKind> {
    lexemes[index + 1..].iter().find_map(|lexeme| match lexeme {
        Lexeme::Token(token) if token.kind != TokenKind::Eof => Some(&token.kind),
        _ => None,
    })
}

fn lexical_kind(
    token: &TokenKind,
    previous: Option<&TokenKind>,
    next: Option<&TokenKind>,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> Option<SemanticTokenKind> {
    match token {
        TokenKind::Ident(name) if matches!(name.as_str(), "true" | "false" | "None") => {
            Some(SemanticTokenKind::Constant)
        }
        TokenKind::Ident(name) if is_keyword(name) => Some(SemanticTokenKind::Keyword),
        TokenKind::Ident(name)
            if primary.is_some_and(|symbol| {
                primary_name(symbol, library) == name && matches!(symbol, StdlibSymbolId::Item(_))
            }) =>
        {
            let StdlibSymbolId::Item(item) = primary.unwrap() else {
                unreachable!()
            };
            Some(match library.item(item).kind {
                crate::stdlib::ItemKind::Function => SemanticTokenKind::Function,
                crate::stdlib::ItemKind::Method { .. } => SemanticTokenKind::Method,
            })
        }
        TokenKind::Ident(name)
            if is_type_name(name, library)
                && (name != "address"
                    || matches!(previous, Some(TokenKind::Colon | TokenKind::Gt))
                    || matches!(next, Some(TokenKind::Dot))
                    || (previous.is_none() && next.is_none())) =>
        {
            Some(SemanticTokenKind::Type)
        }
        TokenKind::Ident(_) if matches!(next, Some(TokenKind::Colon)) => {
            Some(SemanticTokenKind::Parameter)
        }
        TokenKind::Ident(_) if matches!(next, Some(TokenKind::LParen)) => {
            Some(if matches!(previous, Some(TokenKind::Dot)) {
                SemanticTokenKind::Method
            } else {
                SemanticTokenKind::Function
            })
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Dot)) => {
            Some(SemanticTokenKind::Property)
        }
        TokenKind::Ident(_) => Some(SemanticTokenKind::Variable),
        TokenKind::Char(_)
        | TokenKind::String(_)
        | TokenKind::TemplateStart
        | TokenKind::TemplateChunk(_)
        | TokenKind::TemplateEnd => Some(SemanticTokenKind::String),
        TokenKind::DocComment(_) => Some(SemanticTokenKind::Comment),
        TokenKind::Int(_) | TokenKind::Float(_) => Some(SemanticTokenKind::Number),
        kind if is_operator(kind) => Some(SemanticTokenKind::Operator),
        _ => None,
    }
}

fn is_linkable(kind: SemanticTokenKind) -> bool {
    matches!(
        kind,
        SemanticTokenKind::Keyword
            | SemanticTokenKind::Type
            | SemanticTokenKind::Struct
            | SemanticTokenKind::Enum
            | SemanticTokenKind::EnumMember
            | SemanticTokenKind::Function
            | SemanticTokenKind::Method
            | SemanticTokenKind::Property
            | SemanticTokenKind::Namespace
            | SemanticTokenKind::Lifecycle
    )
}

fn lexical_target(
    name: &str,
    previous: Option<&TokenKind>,
    next: Option<&TokenKind>,
    primary: Option<StdlibSymbolId>,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<String> {
    if let Some(primary) = primary
        && primary_name(primary, library) == name
        && (matches!(previous, Some(TokenKind::Dot))
            || !matches!(previous, Some(TokenKind::LParen | TokenKind::Comma)))
    {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(primary, library),
        ));
    }
    if let Some(ty) = library.type_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Type(ty.id), library),
        ));
    }
    if let Some(capability) = library
        .capabilities()
        .iter()
        .find(|capability| capability.name == name)
    {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Capability(capability.id), library),
        ));
    }
    if let Some(namespace) = library.namespace_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Namespace(namespace.id), library),
        ));
    }
    if let Some(constructor) = library.named_type_constructor_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::TypeConstructor(constructor.id), library),
        ));
    }
    let type_position = matches!(previous, Some(TokenKind::Colon | TokenKind::Gt))
        || matches!(next, Some(TokenKind::Dot))
        || (previous.is_none() && next.is_none());
    if type_position && let Some(ty) = library.core_types().iter().find(|ty| ty.name == name) {
        return Some(relative_document_link(
            current_uri,
            &core_type_uri(ty.id, library),
        ));
    }
    if let Some(item) = LanguageCatalog::new().item_for_source_token(name) {
        return Some(relative_document_link(
            current_uri,
            &language_item_uri(item.id),
        ));
    }
    None
}

fn target_uri(
    target: DefinitionTarget,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<String> {
    let target = match target {
        DefinitionTarget::StandardLibrary(item) => symbol_uri(StdlibSymbolId::Item(item), library),
        DefinitionTarget::StandardLibrarySymbol(symbol) => symbol_uri(symbol, library),
        DefinitionTarget::Language(id) => language_item_uri(id),
        DefinitionTarget::Source(_) => return None,
    };
    Some(relative_document_link(current_uri, &target))
}

fn primary_name(symbol: StdlibSymbolId, library: &StandardLibrary) -> &'static str {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name,
        StdlibSymbolId::Namespace(id) => library.namespace(id).name,
        StdlibSymbolId::Capability(id) => library.capability(id).name,
        StdlibSymbolId::TypeConstructor(id) => library.type_constructor(id).name,
        StdlibSymbolId::Type(id) => library.type_decl(id).name,
        StdlibSymbolId::Field(id) => library.field(id).name,
        StdlibSymbolId::Variant(id) => library.variant(id).name,
        StdlibSymbolId::Item(id) => library.item(id).name,
    }
}

fn is_type_name(name: &str, library: &StandardLibrary) -> bool {
    library.core_types().iter().any(|ty| ty.name == name)
        || library.type_by_name(name).is_some()
        || library.named_type_constructor_by_name(name).is_some()
        || library
            .capabilities()
            .iter()
            .any(|capability| capability.name == name)
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "state"
            | "tickRate"
            | "layout"
            | "settings"
            | "record"
            | "enum"
            | "fn"
            | "let"
            | "if"
            | "else"
            | "while"
            | "for"
            | "in"
            | "break"
            | "continue"
            | "return"
            | "throw"
            | "async"
            | "await"
            | "retry"
            | "match"
            | "as"
            | "where"
            | "Some"
            | "Ok"
            | "Err"
    )
}

fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::FatArrow
            | TokenKind::Assign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::PercentAssign
            | TokenKind::OrAssign
            | TokenKind::AndAssign
            | TokenKind::CaretAssign
            | TokenKind::ShlAssign
            | TokenKind::ShrAssign
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Bang
            | TokenKind::Question
            | TokenKind::Tilde
            | TokenKind::Or
            | TokenKind::And
            | TokenKind::Caret
            | TokenKind::OrOr
            | TokenKind::AndAnd
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Shl
            | TokenKind::Shr
            | TokenKind::Dot
            | TokenKind::DotDotEq
    )
}

fn render(source: &str, annotations: &[Annotation]) -> String {
    let mut html = String::from("<pre class=\"hljs splitscript-code\"><code>");
    let mut cursor = 0;
    for annotation in annotations {
        if annotation.start < cursor
            || annotation.start > annotation.end
            || annotation.end > source.len()
        {
            continue;
        }
        escape_html_into(&mut html, &source[cursor..annotation.start]);
        if let Some(target) = &annotation.target {
            html.push_str("<a href=\"");
            escape_html_attribute_into(&mut html, target);
            html.push_str("\">");
        }
        html.push_str("<span class=\"");
        html.push_str(css_class(annotation.kind));
        html.push_str("\">");
        escape_html_into(&mut html, &source[annotation.start..annotation.end]);
        html.push_str("</span>");
        if annotation.target.is_some() {
            html.push_str("</a>");
        }
        cursor = annotation.end;
    }
    escape_html_into(&mut html, &source[cursor..]);
    html.push_str("</code></pre>");
    html
}

fn css_class(kind: SemanticTokenKind) -> &'static str {
    match kind {
        SemanticTokenKind::Keyword | SemanticTokenKind::Debug => "hljs-keyword",
        SemanticTokenKind::Type | SemanticTokenKind::Struct | SemanticTokenKind::Enum => {
            "hljs-type"
        }
        SemanticTokenKind::EnumMember | SemanticTokenKind::Constant => "hljs-literal",
        SemanticTokenKind::Function | SemanticTokenKind::Method | SemanticTokenKind::Lifecycle => {
            "hljs-title function_"
        }
        SemanticTokenKind::Variable | SemanticTokenKind::Parameter => "hljs-variable",
        SemanticTokenKind::Property
        | SemanticTokenKind::Setting
        | SemanticTokenKind::SettingTitle
        | SemanticTokenKind::StateField => "hljs-attr",
        SemanticTokenKind::Namespace => "hljs-built_in",
        SemanticTokenKind::String | SemanticTokenKind::Signature | SemanticTokenKind::Version => {
            "hljs-string"
        }
        SemanticTokenKind::Number => "hljs-number",
        SemanticTokenKind::Operator => "splitscript-token-operator",
        SemanticTokenKind::Comment => "hljs-comment",
    }
}

fn escape_html_into(output: &mut String, source: &str) {
    for character in source.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

fn escape_html_attribute_into(output: &mut String, source: &str) {
    escape_html_into(output, source);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::StdlibItemId;

    #[test]
    fn signatures_highlight_and_link_catalog_symbols() {
        let library = StandardLibrary::new();
        let item = library.item(StdlibItemId::ProcessRead);
        let source = library.render_signature(item.id);
        let html = signature(
            &source,
            "/stdlib/types/Process/methods/read.md",
            Some(StdlibSymbolId::Item(item.id)),
            &library,
        );
        assert!(
            html.contains("class=\"hljs-type\">Process</span>"),
            "{html}"
        );
        assert!(
            html.contains("href=\"read.md\"><span class=\"hljs-title function_\">read"),
            "{html}"
        );
        assert!(
            html.contains("capabilities/MemoryReadable/index.md"),
            "{html}"
        );
    }

    #[test]
    fn semantic_examples_link_resolved_methods_but_not_fixture_locals() {
        let library = StandardLibrary::new();
        let documentation_example = Example::checked(
            "Read a value",
            "let health = process.read<i32>(player.offset(0x20)) else 0",
            "state \"game.exe\" {}\nonAttach {\nlet player: address = 0x1000\nlet health = process.read<i32>(player.offset(0x20)) else 0\nprint(health)\n}",
        );
        let html = example(
            documentation_example,
            "/stdlib/types/Process/methods/read.md",
            &library,
        );
        assert!(html.contains("href=\"read.md"), "{html}");
        assert!(html.contains("address/methods/offset.md"), "{html}");
        assert!(!html.contains("href=\"player"));
    }

    #[test]
    fn semantic_examples_link_language_keywords() {
        let library = StandardLibrary::new();
        let documentation_example = Example::complete_program(
            "Load asynchronously",
            "fn load() -> async Module {\n    let module = await process.module(\"game.dll\")\n    return module\n}",
        );
        let html = example(documentation_example, "/language/async.md", &library);
        for keyword in ["fn", "async", "let", "await", "return"] {
            assert!(
                html.contains(&format!("href=\"{keyword}.md\"")),
                "missing link for `{keyword}` in {html}"
            );
        }
    }
}
