use crate::{Token, TokenKind};

/// Shared navigation over one token stream.
///
/// The cursor never advances past EOF, and lookahead saturates at the final
/// token. Grammar-specific parsers retain ownership of diagnostics and
/// recovery decisions while sharing these low-level invariants.
#[derive(Debug, Clone)]
pub struct TokenCursor {
    tokens: Vec<Token>,
    position: usize,
}

impl TokenCursor {
    pub fn new(tokens: Vec<Token>) -> Self {
        assert!(
            matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Eof)),
            "a parser token stream must end in EOF"
        );
        Self {
            tokens,
            position: 0,
        }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    pub fn previous(&self) -> &Token {
        &self.tokens[self.position.saturating_sub(1)]
    }

    pub fn peek(&self, offset: usize) -> &Token {
        &self.tokens[self
            .position
            .saturating_add(offset)
            .min(self.tokens.len() - 1)]
    }

    pub fn bump(&mut self) -> &Token {
        let index = self.position;
        if !matches!(self.tokens[index].kind, TokenKind::Eof) {
            self.position += 1;
        }
        &self.tokens[index]
    }

    pub fn at(&self, expected: &TokenKind) -> bool {
        self.current().kind == *expected
    }

    pub fn at_variant(&self, expected: &TokenKind) -> bool {
        std::mem::discriminant(&self.current().kind) == std::mem::discriminant(expected)
    }

    pub fn at_ident(&self, expected: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Ident(value) if value == expected)
    }

    pub fn take_doc_comments(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while let TokenKind::DocComment(line) = self.current().kind.clone() {
            lines.push(line);
            self.bump();
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use crate::{Span, SyntaxMode, lex};

    use super::*;

    #[test]
    fn lookahead_and_bumping_saturate_at_eof() {
        let mut cursor = TokenCursor::new(lex("value", SyntaxMode::Program).unwrap());
        assert!(cursor.at_ident("value"));
        assert_eq!(cursor.peek(usize::MAX).kind, TokenKind::Eof);
        assert_eq!(cursor.bump().span, Span { start: 0, end: 5 });
        assert_eq!(cursor.position(), 1);
        assert_eq!(cursor.bump().kind, TokenKind::Eof);
        assert_eq!(cursor.bump().kind, TokenKind::Eof);
        assert_eq!(cursor.position(), 1);
        assert_eq!(cursor.previous().kind, TokenKind::Ident("value".to_owned()));
    }

    #[test]
    fn exact_and_variant_matching_are_explicit() {
        let cursor = TokenCursor::new(lex("actual", SyntaxMode::Program).unwrap());
        assert!(!cursor.at(&TokenKind::Ident("other".to_owned())));
        assert!(cursor.at_variant(&TokenKind::Ident(String::new())));
    }

    #[test]
    fn documentation_blocks_are_consumed_without_eating_the_declaration() {
        let mut cursor =
            TokenCursor::new(lex("/// First\n///\n/// Second\nfn", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.take_doc_comments(), ["First", "", "Second"]);
        assert!(cursor.at_ident("fn"));
    }
}
