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
    previous: Option<Token>,
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
            previous: None,
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
        self.previous.as_ref().unwrap_or_else(|| self.current())
    }

    pub fn peek(&self, offset: usize) -> &Token {
        &self.tokens[self
            .position
            .saturating_add(offset)
            .min(self.tokens.len() - 1)]
    }

    pub fn bump(&mut self) -> &Token {
        let index = self.position;
        if matches!(self.tokens[index].kind, TokenKind::Eof) {
            return &self.tokens[index];
        }
        self.position += 1;
        self.previous = Some(self.tokens[index].clone());
        self.previous.as_ref().unwrap()
    }

    /// Consumes one leading `>` from the current token.
    ///
    /// The lexer deliberately uses maximal munch for comparison and shift
    /// operators. Type grammars nevertheless need to read adjacent generic
    /// closers and a following assignment without making whitespace semantic.
    /// This method exposes one source-accurate `>` and leaves any remaining
    /// `>`, `>=`, or `=` as the current token for the surrounding grammar.
    pub fn eat_leading_gt(&mut self) -> Option<crate::Span> {
        let token = self.current().clone();
        let close = crate::Span {
            start: token.span.start,
            end: token.span.start + 1,
        };
        let residual = match token.kind {
            TokenKind::Gt => {
                self.bump();
                return Some(close);
            }
            TokenKind::Ge => Token {
                kind: TokenKind::Assign,
                span: crate::Span {
                    start: close.end,
                    end: token.span.end,
                },
            },
            TokenKind::Shr => Token {
                kind: TokenKind::Gt,
                span: crate::Span {
                    start: close.end,
                    end: token.span.end,
                },
            },
            TokenKind::ShrAssign => Token {
                kind: TokenKind::Ge,
                span: crate::Span {
                    start: close.end,
                    end: token.span.end,
                },
            },
            _ => return None,
        };

        self.set_residual(residual);
        self.previous = Some(Token {
            kind: TokenKind::Gt,
            span: close,
        });
        Some(close)
    }

    /// Consumes a type-level `!` even when maximal munch combined it with a
    /// following assignment into `!=`.
    pub fn eat_leading_bang(&mut self) -> Option<crate::Span> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Bang => {
                let span = self.bump().span;
                Some(span)
            }
            TokenKind::BangEq if token.span.end - token.span.start == 2 => {
                let bang = crate::Span {
                    start: token.span.start,
                    end: token.span.start + 1,
                };
                self.set_residual(Token {
                    kind: TokenKind::Assign,
                    span: crate::Span {
                        start: bang.end,
                        end: token.span.end,
                    },
                });
                self.previous = Some(Token {
                    kind: TokenKind::Bang,
                    span: bang,
                });
                Some(bang)
            }
            _ => None,
        }
    }

    /// Rebuilds an operator that maximal munch had split across the contextual
    /// boundary. For example, `T>==value` lexes as `>=` followed by `=`;
    /// consuming the type's `>` must expose one logical `==`, not two
    /// assignments.
    fn set_residual(&mut self, mut residual: Token) {
        if let Some(next) = self.tokens.get(self.position + 1)
            && residual.span.end == next.span.start
        {
            let combined = match (&residual.kind, &next.kind) {
                (TokenKind::Assign, TokenKind::Assign) => Some(TokenKind::EqEq),
                (TokenKind::Assign, TokenKind::EqEq) => Some(TokenKind::EqEq),
                (TokenKind::Gt, TokenKind::Gt) => Some(TokenKind::Shr),
                (TokenKind::Gt, TokenKind::Ge) => Some(TokenKind::ShrAssign),
                _ => None,
            };
            if let Some(kind) = combined {
                residual.kind = kind;
                residual.span.end = next.span.end;
                self.tokens.remove(self.position + 1);
            }
        }
        self.tokens[self.position] = residual;
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

    #[test]
    fn generic_closers_are_fissioned_without_changing_operator_lexing() {
        let mut cursor = TokenCursor::new(lex(">>=", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.current().kind, TokenKind::ShrAssign);

        assert_eq!(cursor.eat_leading_gt(), Some(Span { start: 0, end: 1 }));
        assert_eq!(cursor.previous().kind, TokenKind::Gt);
        assert_eq!(cursor.current().kind, TokenKind::Ge);
        assert_eq!(cursor.current().span, Span { start: 1, end: 3 });

        assert_eq!(cursor.eat_leading_gt(), Some(Span { start: 1, end: 2 }));
        assert_eq!(cursor.previous().kind, TokenKind::Gt);
        assert_eq!(cursor.current().kind, TokenKind::Assign);
        assert_eq!(cursor.current().span, Span { start: 2, end: 3 });

        cursor.bump();
        assert_eq!(cursor.previous().kind, TokenKind::Assign);
        assert_eq!(cursor.current().kind, TokenKind::Eof);
    }

    #[test]
    fn equality_is_reassembled_after_a_contextual_generic_close() {
        let mut cursor = TokenCursor::new(lex(">==", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.current().kind, TokenKind::Ge);
        assert_eq!(cursor.eat_leading_gt(), Some(Span { start: 0, end: 1 }));
        assert_eq!(cursor.current().kind, TokenKind::EqEq);
        assert_eq!(cursor.current().span, Span { start: 1, end: 3 });

        let mut nested = TokenCursor::new(lex(">>==", SyntaxMode::Program).unwrap());
        assert_eq!(nested.current().kind, TokenKind::ShrAssign);
        nested.eat_leading_gt().unwrap();
        nested.eat_leading_gt().unwrap();
        assert_eq!(nested.current().kind, TokenKind::EqEq);
        assert_eq!(nested.current().span, Span { start: 2, end: 4 });

        let mut strict = TokenCursor::new(lex(">===", SyntaxMode::Program).unwrap());
        strict.eat_leading_gt().unwrap();
        assert_eq!(strict.current().kind, TokenKind::EqEq);
        assert_eq!(strict.current().span, Span { start: 1, end: 4 });
    }

    #[test]
    fn shift_is_reassembled_after_a_contextual_generic_close() {
        let mut cursor = TokenCursor::new(lex(">>>", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.current().kind, TokenKind::Shr);
        cursor.eat_leading_gt().unwrap();
        assert_eq!(cursor.current().kind, TokenKind::Shr);
        assert_eq!(cursor.current().span, Span { start: 1, end: 3 });
    }

    #[test]
    fn three_generic_closers_leave_a_following_assignment() {
        let mut cursor = TokenCursor::new(lex(">>>=", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.current().kind, TokenKind::Shr);
        for expected_start in 0..3 {
            assert_eq!(
                cursor.eat_leading_gt(),
                Some(Span {
                    start: expected_start,
                    end: expected_start + 1,
                })
            );
        }
        assert_eq!(cursor.current().kind, TokenKind::Assign);
        assert_eq!(cursor.current().span, Span { start: 3, end: 4 });
    }

    #[test]
    fn fallible_type_suffix_is_fissioned_from_assignment() {
        let mut cursor = TokenCursor::new(lex("!=", SyntaxMode::Program).unwrap());
        assert_eq!(cursor.current().kind, TokenKind::BangEq);
        assert_eq!(cursor.eat_leading_bang(), Some(Span { start: 0, end: 1 }));
        assert_eq!(cursor.previous().kind, TokenKind::Bang);
        assert_eq!(cursor.current().kind, TokenKind::Assign);
        assert_eq!(cursor.current().span, Span { start: 1, end: 2 });
    }
}
