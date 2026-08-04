//! Lossless source text and token/trivia access for formatter and editor tools.

use crate::{Lexed, Lexeme, Span, Token, Trivia};

#[derive(Debug, Clone, PartialEq)]
pub struct SourceDocument {
    source: String,
    lexemes: Vec<Lexeme>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryNodeKind {
    Missing,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryNode {
    pub kind: RecoveryNodeKind,
    pub span: Span,
}

impl SourceDocument {
    /// Builds a lossless document from source and its matching lexer output.
    pub fn from_lexed(source: &str, lexed: Lexed) -> Self {
        Self {
            source: source.to_owned(),
            lexemes: lexed.into_lexemes(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn lexemes(&self) -> &[Lexeme] {
        &self.lexemes
    }

    pub fn tokens(&self) -> impl Iterator<Item = &Token> {
        self.lexemes.iter().filter_map(|lexeme| match lexeme {
            Lexeme::Token(token) => Some(token),
            Lexeme::Trivia(_) => None,
        })
    }

    /// Returns the non-empty token containing `offset`.
    ///
    /// Spans are half-open, so an offset at the end of one token belongs to a
    /// following token only when that token starts at the same byte. Trivia is
    /// intentionally not skipped: an offset in whitespace or a comment has no
    /// token.
    pub fn token_at(&self, offset: usize) -> Option<&Token> {
        self.lexemes.iter().find_map(|lexeme| match lexeme {
            Lexeme::Token(token)
                if token.span.start <= offset
                    && offset < token.span.end
                    && token.span.start != token.span.end =>
            {
                Some(token)
            }
            Lexeme::Token(_) | Lexeme::Trivia(_) => None,
        })
    }

    /// Returns the token selected by an editor cursor.
    ///
    /// Editor cursors commonly sit at a symbol's half-open end. An exact token
    /// at that offset always wins, including punctuation that begins where the
    /// preceding word ends. Only a genuine token gap or end of file falls back
    /// to the immediately preceding word-like token when it ends exactly at the
    /// cursor. Whitespace farther away from a token remains unselected.
    pub fn symbol_token_at(&self, offset: usize) -> Option<&Token> {
        if let Some(current) = self.token_at(offset) {
            return Some(current);
        }
        offset
            .checked_sub(1)
            .and_then(|previous| self.token_at(previous))
            .filter(|token| token.span.end == offset && is_word_like(&token.kind))
    }

    pub fn trivia(&self) -> impl Iterator<Item = &Trivia> {
        self.lexemes.iter().filter_map(|lexeme| match lexeme {
            Lexeme::Trivia(trivia) => Some(trivia),
            Lexeme::Token(_) => None,
        })
    }

    pub fn text(&self, span: Span) -> &str {
        &self.source[span.start..span.end]
    }

    /// Reconstructs the source from its non-empty token and trivia spans.
    /// This is primarily a losslessness invariant for tooling tests.
    pub fn reconstruct(&self) -> String {
        let mut reconstructed = String::with_capacity(self.source.len());
        for lexeme in &self.lexemes {
            let span = lexeme.span();
            if span.start != span.end {
                reconstructed.push_str(self.text(span));
            }
        }
        reconstructed
    }
}

fn is_word_like(kind: &crate::TokenKind) -> bool {
    matches!(
        kind,
        crate::TokenKind::Ident(_)
            | crate::TokenKind::Int(_)
            | crate::TokenKind::Float(_)
            | crate::TokenKind::String(_)
            | crate::TokenKind::DocComment(_)
            | crate::TokenKind::TemplateChunk(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SyntaxMode, TokenKind, TriviaKind, lex_lossless};

    #[test]
    fn reconstructs_every_token_comment_and_whitespace_byte() {
        let source = "  // heading\r\n/// setting help\r\nname /* middle */ = `a {1 + 2}`\n";
        let document =
            SourceDocument::from_lexed(source, lex_lossless(source, SyntaxMode::Program).unwrap());

        assert_eq!(document.reconstruct(), source);
        assert!(document.trivia().any(|trivia| {
            trivia.kind == TriviaKind::LineComment && document.text(trivia.span) == "// heading"
        }));
        assert!(document.trivia().any(|trivia| {
            trivia.kind == TriviaKind::BlockComment && document.text(trivia.span) == "/* middle */"
        }));
        assert!(document.tokens().any(|token| {
            matches!(&token.kind, TokenKind::DocComment(text) if text == "setting help")
                && document.text(token.span) == "/// setting help"
        }));

        let mut end = 0;
        for lexeme in document.lexemes() {
            let span = lexeme.span();
            assert_eq!(span.start, end);
            end = span.end;
        }
        assert_eq!(end, source.len());
    }

    #[test]
    fn finds_only_the_token_directly_under_an_offset() {
        let source = "alpha /* gap */ . beta";
        let document =
            SourceDocument::from_lexed(source, lex_lossless(source, SyntaxMode::Program).unwrap());

        let alpha = document.token_at(2).unwrap();
        assert!(matches!(&alpha.kind, TokenKind::Ident(name) if name == "alpha"));
        assert_eq!(document.text(alpha.span), "alpha");

        let dot = document.token_at(source.find('.').unwrap()).unwrap();
        assert_eq!(dot.kind, TokenKind::Dot);
        assert!(document.token_at(source.find("gap").unwrap()).is_none());
        assert!(document.token_at(source.len()).is_none());
    }

    #[test]
    fn selects_symbols_at_editor_cursor_boundaries() {
        let source = "alpha.beta  gamma(delta) value? result! omega";
        let document =
            SourceDocument::from_lexed(source, lex_lossless(source, SyntaxMode::Program).unwrap());

        let alpha_end = "alpha".len();
        assert_eq!(
            document.text(document.symbol_token_at(alpha_end).unwrap().span),
            "."
        );
        let beta_end = source.find("beta").unwrap() + "beta".len();
        assert_eq!(
            document.text(document.symbol_token_at(beta_end).unwrap().span),
            "beta"
        );
        assert!(document.symbol_token_at(beta_end + 1).is_none());
        let gamma = source.find("gamma").unwrap();
        assert_eq!(
            document.text(document.symbol_token_at(gamma).unwrap().span),
            "gamma"
        );

        let gamma_end = gamma + "gamma".len();
        assert_eq!(
            document.text(document.symbol_token_at(gamma_end).unwrap().span),
            "("
        );
        let delta_end = source.find("delta").unwrap() + "delta".len();
        assert_eq!(
            document.text(document.symbol_token_at(delta_end).unwrap().span),
            ")"
        );

        for punctuation in ['?', '!'] {
            let offset = source.find(punctuation).unwrap();
            assert_eq!(
                document.text(document.symbol_token_at(offset).unwrap().span),
                punctuation.to_string()
            );
        }

        let omega_end = source.len();
        assert_eq!(
            document.text(document.symbol_token_at(omega_end).unwrap().span),
            "omega"
        );

        let source = "lineEnd\noneSpace \nnext";
        let document =
            SourceDocument::from_lexed(source, lex_lossless(source, SyntaxMode::Program).unwrap());
        let line_end = "lineEnd".len();
        assert_eq!(
            document.text(document.symbol_token_at(line_end).unwrap().span),
            "lineEnd"
        );
        let one_space_end = source.find("oneSpace").unwrap() + "oneSpace".len();
        assert_eq!(
            document.text(document.symbol_token_at(one_space_end).unwrap().span),
            "oneSpace"
        );
        assert!(document.symbol_token_at(one_space_end + 1).is_none());
    }
}
