//! Compiler-facing adapter for the shared SplitScript lexer.

use splitscript_syntax::SyntaxMode;
pub use splitscript_syntax::{Lexed, Lexeme, Token, TokenKind, TriviaKind};

use crate::Diagnostic;

#[cfg(test)]
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    splitscript_syntax::lex(source, SyntaxMode::Program).map_err(into_diagnostic)
}

pub fn lex_lossless(source: &str) -> Result<Lexed, Diagnostic> {
    splitscript_syntax::lex_lossless(source, SyntaxMode::Program).map_err(into_diagnostic)
}

/// Produces an offset-preserving token stream for editor recovery even when
/// strict lexing encounters malformed text. The shared lexer resumes from the
/// affected token boundary after replacing malformed bytes with same-width
/// whitespace, so valid regions keep their exact original offsets without
/// rescanning the complete prefix. Strict compilation still rejects every
/// collected lexical error.
pub fn lex_lossless_recovering(source: &str) -> (Lexed, Vec<Diagnostic>) {
    let (lexed, errors) = splitscript_syntax::lex_lossless_recovering(source, SyntaxMode::Program);
    (lexed, errors.into_iter().map(into_diagnostic).collect())
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
