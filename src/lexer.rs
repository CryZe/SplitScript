//! Compiler-facing adapter for the shared SplitScript lexer.

use splitscript_syntax::SyntaxMode;
pub use splitscript_syntax::{Lexed, Lexeme, Token, TokenKind, TriviaKind};

use crate::{Diagnostic, ast::Span};

#[cfg(test)]
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    splitscript_syntax::lex(source, SyntaxMode::Program).map_err(into_diagnostic)
}

pub fn lex_lossless(source: &str) -> Result<Lexed, Diagnostic> {
    splitscript_syntax::lex_lossless(source, SyntaxMode::Program).map_err(into_diagnostic)
}

/// Produces an offset-preserving token stream for editor recovery even when
/// strict lexing encounters malformed text. Each failing source span is
/// replaced by same-width whitespace in a private probe buffer, so tokens from
/// valid regions keep their exact original offsets. Strict compilation keeps
/// using [`lex_lossless`] and therefore still rejects every lexical error.
pub fn lex_lossless_recovering(source: &str) -> (Lexed, Vec<Diagnostic>) {
    let mut probe = source.as_bytes().to_vec();
    let mut diagnostics = Vec::new();
    loop {
        let probe_source = std::str::from_utf8(&probe)
            .expect("offset-preserving lexical repairs retain valid UTF-8");
        match splitscript_syntax::lex_lossless(probe_source, SyntaxMode::Program) {
            Ok(lexed) => return (lexed, diagnostics),
            Err(error) => {
                let span = lexical_repair_span(source, error.span);
                diagnostics.push(Diagnostic::lexical(error.message, span));
                for (offset, byte) in probe[span.start..span.end].iter_mut().enumerate() {
                    if !matches!(source.as_bytes()[span.start + offset], b'\r' | b'\n') {
                        *byte = b' ';
                    }
                }
            }
        }
    }
}

fn lexical_repair_span(source: &str, span: Span) -> Span {
    let mut start = span.start.min(source.len());
    while start > 0 && !source.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = span.end.max(start.saturating_add(1)).min(source.len());
    while end < source.len() && !source.is_char_boundary(end) {
        end += 1;
    }
    if start == end {
        start = source[..start]
            .char_indices()
            .next_back()
            .map_or(0, |(offset, _)| offset);
    }
    Span { start, end }
}

fn into_diagnostic(error: splitscript_syntax::Error) -> Diagnostic {
    Diagnostic::lexical(error.message, error.span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::SourceDocument;

    #[test]
    fn recovering_lexing_retains_valid_regions_and_original_offsets() {
        let source = "fn before() {}\nlet broken = \"unfinished\nfn after() {}\n";
        let (lexed, diagnostics) = lex_lossless_recovering(source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "unterminated string literal")
        );

        let document = SourceDocument::from_lexed(source, lexed);
        assert_eq!(document.reconstruct(), source);
        assert!(document.tokens().any(|token| {
            token.span.start == source.find("after").unwrap()
                && token.kind == TokenKind::Ident("after".to_owned())
        }));
    }

    #[test]
    fn recovering_lexing_replaces_complete_unicode_scalars() {
        let source = "fn before() {}\n🦊\nfn after() {}\n";
        let (lexed, diagnostics) = lex_lossless_recovering(source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "unexpected character")
        );

        let document = SourceDocument::from_lexed(source, lexed);
        assert_eq!(document.reconstruct(), source);
        assert!(document.tokens().any(|token| {
            token.span.start == source.find("after").unwrap()
                && token.kind == TokenKind::Ident("after".to_owned())
        }));
    }
}
