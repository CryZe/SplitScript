use crate::{Diagnostic, ast::Span};

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriviaKind {
    Whitespace,
    LineComment,
    BlockComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trivia {
    pub kind: TriviaKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lexeme {
    Token(Token),
    Trivia(Trivia),
}

impl Lexeme {
    pub fn span(&self) -> Span {
        match self {
            Self::Token(token) => token.span,
            Self::Trivia(trivia) => trivia.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lexed {
    lexemes: Vec<Lexeme>,
}

impl Lexed {
    pub fn tokens(&self) -> impl Iterator<Item = &Token> {
        self.lexemes.iter().filter_map(|lexeme| match lexeme {
            Lexeme::Token(token) => Some(token),
            Lexeme::Trivia(_) => None,
        })
    }

    pub fn into_lexemes(self) -> Vec<Lexeme> {
        self.lexemes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Int(String),
    Float(String),
    String(String),
    DocComment(String),
    TemplateStart,
    TemplateChunk(String),
    TemplateExprStart,
    TemplateExprEnd,
    TemplateEnd,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Semicolon,
    Dot,
    FatArrow,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    OrAssign,
    AndAssign,
    CaretAssign,
    ShlAssign,
    ShrAssign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Question,
    Tilde,
    Or,
    And,
    Caret,
    OrOr,
    AndAnd,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    Eof,
}

#[cfg(test)]
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    Ok(lex_lossless(source)?.tokens().cloned().collect())
}

pub fn lex_lossless(source: &str) -> Result<Lexed, Diagnostic> {
    Lexer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        modes: vec![LexMode::Code],
    }
    .run()
}

#[derive(Debug, Clone, Copy)]
enum LexMode {
    Code,
    Template { start: usize },
    Interpolation { start: usize, brace_depth: u32 },
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    modes: Vec<LexMode>,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Lexed, Diagnostic> {
        let mut lexemes = Vec::new();
        loop {
            if matches!(self.modes.last(), Some(LexMode::Template { .. })) {
                lexemes.push(Lexeme::Token(self.template_token()?));
                continue;
            }
            if let Some(token) = self.skip_trivia(&mut lexemes)? {
                lexemes.push(Lexeme::Token(token));
                continue;
            }
            let start = self.pos;
            if self.pos == self.bytes.len() {
                if let Some(mode) = self.modes.last()
                    && !matches!(mode, LexMode::Code)
                {
                    let start = match mode {
                        LexMode::Template { start } | LexMode::Interpolation { start, .. } => {
                            *start
                        }
                        LexMode::Code => unreachable!(),
                    };
                    return Err(Diagnostic::lexical(
                        "unterminated template literal",
                        Span {
                            start,
                            end: self.pos,
                        },
                    ));
                }
                lexemes.push(Lexeme::Token(Token {
                    kind: TokenKind::Eof,
                    span: Span { start, end: start },
                }));
                return Ok(Lexed { lexemes });
            }

            let kind = match self.bytes[self.pos] {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => self.identifier(),
                b'0'..=b'9' => self.number()?,
                b'"' | b'\'' => self.string()?,
                b'`' => {
                    self.pos += 1;
                    self.modes.push(LexMode::Template { start });
                    TokenKind::TemplateStart
                }
                b'(' => self.one(TokenKind::LParen),
                b')' => self.one(TokenKind::RParen),
                b'{' => {
                    if let Some(LexMode::Interpolation { brace_depth, .. }) = self.modes.last_mut()
                    {
                        *brace_depth += 1;
                    }
                    self.one(TokenKind::LBrace)
                }
                b'}' => {
                    let ends_interpolation = matches!(
                        self.modes.last(),
                        Some(LexMode::Interpolation { brace_depth: 0, .. })
                    );
                    if ends_interpolation {
                        self.pos += 1;
                        self.modes.pop();
                        TokenKind::TemplateExprEnd
                    } else {
                        if let Some(LexMode::Interpolation { brace_depth, .. }) =
                            self.modes.last_mut()
                        {
                            *brace_depth -= 1;
                        }
                        self.one(TokenKind::RBrace)
                    }
                }
                b'[' => self.one(TokenKind::LBracket),
                b']' => self.one(TokenKind::RBracket),
                b':' => self.one(TokenKind::Colon),
                b',' => self.one(TokenKind::Comma),
                b';' => self.one(TokenKind::Semicolon),
                b'.' => self.one(TokenKind::Dot),
                b'+' if self.starts_with(b"+=") => self.many(2, TokenKind::PlusAssign),
                b'+' => self.one(TokenKind::Plus),
                b'-' if self.starts_with(b"-=") => self.many(2, TokenKind::MinusAssign),
                b'-' => self.one(TokenKind::Minus),
                b'*' if self.starts_with(b"*=") => self.many(2, TokenKind::StarAssign),
                b'*' => self.one(TokenKind::Star),
                b'/' if self.starts_with(b"/=") => self.many(2, TokenKind::SlashAssign),
                b'/' => self.one(TokenKind::Slash),
                b'%' if self.starts_with(b"%=") => self.many(2, TokenKind::PercentAssign),
                b'%' => self.one(TokenKind::Percent),
                b'~' => self.one(TokenKind::Tilde),
                b'^' if self.starts_with(b"^=") => self.many(2, TokenKind::CaretAssign),
                b'^' => self.one(TokenKind::Caret),
                b'=' if self.starts_with(b"=>") => self.many(2, TokenKind::FatArrow),
                b'=' if self.starts_with(b"==") => self.many(2, TokenKind::EqEq),
                b'=' => self.one(TokenKind::Assign),
                b'!' if self.starts_with(b"!=") => self.many(2, TokenKind::BangEq),
                b'!' => self.one(TokenKind::Bang),
                b'?' => self.one(TokenKind::Question),
                b'|' if self.starts_with(b"|=") => self.many(2, TokenKind::OrAssign),
                b'|' if self.starts_with(b"||") => self.many(2, TokenKind::OrOr),
                b'|' => self.one(TokenKind::Or),
                b'&' if self.starts_with(b"&=") => self.many(2, TokenKind::AndAssign),
                b'&' if self.starts_with(b"&&") => self.many(2, TokenKind::AndAnd),
                b'&' => self.one(TokenKind::And),
                b'<' if self.starts_with(b"<<=") => self.many(3, TokenKind::ShlAssign),
                b'<' if self.starts_with(b"<<") => self.many(2, TokenKind::Shl),
                b'<' if self.starts_with(b"<=") => self.many(2, TokenKind::Le),
                b'<' => self.one(TokenKind::Lt),
                b'>' if self.starts_with(b">>=") => self.many(3, TokenKind::ShrAssign),
                b'>' if self.starts_with(b">>") => self.many(2, TokenKind::Shr),
                b'>' if self.starts_with(b">=") => self.many(2, TokenKind::Ge),
                b'>' => self.one(TokenKind::Gt),
                _ => {
                    let end = (self.pos + 1).min(self.bytes.len());
                    return Err(Diagnostic::lexical(
                        "unexpected character",
                        Span { start, end },
                    ));
                }
            };
            lexemes.push(Lexeme::Token(Token {
                kind,
                span: Span {
                    start,
                    end: self.pos,
                },
            }));
        }
    }

    fn template_token(&mut self) -> Result<Token, Diagnostic> {
        let template_start = match self.modes.last() {
            Some(LexMode::Template { start }) => *start,
            _ => unreachable!(),
        };
        let start = self.pos;
        let mut value = String::new();
        while let Some(&byte) = self.bytes.get(self.pos) {
            if byte == b'`' {
                if !value.is_empty() {
                    return Ok(Token {
                        kind: TokenKind::TemplateChunk(value),
                        span: Span {
                            start,
                            end: self.pos,
                        },
                    });
                }
                self.pos += 1;
                self.modes.pop();
                return Ok(Token {
                    kind: TokenKind::TemplateEnd,
                    span: Span {
                        start,
                        end: self.pos,
                    },
                });
            }
            if byte == b'{' {
                if !value.is_empty() {
                    return Ok(Token {
                        kind: TokenKind::TemplateChunk(value),
                        span: Span {
                            start,
                            end: self.pos,
                        },
                    });
                }
                self.pos += 1;
                self.modes.push(LexMode::Interpolation {
                    start: self.pos - 1,
                    brace_depth: 0,
                });
                return Ok(Token {
                    kind: TokenKind::TemplateExprStart,
                    span: Span {
                        start,
                        end: self.pos,
                    },
                });
            }
            if byte == b'}' {
                return Err(Diagnostic::lexical(
                    "unmatched `}` in template literal; write `\\}` for a literal brace",
                    Span {
                        start: self.pos,
                        end: self.pos + 1,
                    },
                ));
            }
            if byte == b'\\' {
                self.pos += 1;
                let Some(&escaped) = self.bytes.get(self.pos) else {
                    break;
                };
                value.push(match escaped {
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'\\' => '\\',
                    b'`' => '`',
                    b'$' => '$',
                    b'{' => '{',
                    b'}' => '}',
                    b'"' => '"',
                    b'\'' => '\'',
                    _ => {
                        return Err(Diagnostic::lexical(
                            "unsupported template string escape",
                            Span {
                                start: self.pos - 1,
                                end: self.pos + 1,
                            },
                        ));
                    }
                });
                self.pos += 1;
            } else if byte.is_ascii() {
                value.push(byte as char);
                self.pos += 1;
            } else {
                let ch = self.source[self.pos..].chars().next().unwrap();
                value.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err(Diagnostic::lexical(
            "unterminated template literal",
            Span {
                start: template_start,
                end: self.pos,
            },
        ))
    }

    fn skip_trivia(&mut self, lexemes: &mut Vec<Lexeme>) -> Result<Option<Token>, Diagnostic> {
        loop {
            let whitespace_start = self.pos;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                self.pos += 1;
            }
            if self.pos != whitespace_start {
                lexemes.push(Lexeme::Trivia(Trivia {
                    kind: TriviaKind::Whitespace,
                    span: Span {
                        start: whitespace_start,
                        end: self.pos,
                    },
                }));
            }
            if self.starts_with(b"///") && !self.starts_with(b"////") {
                let start = self.pos;
                self.pos += 3;
                let content_start = self.pos;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
                {
                    self.pos += 1;
                }
                return Ok(Some(Token {
                    kind: TokenKind::DocComment(
                        self.source[content_start..self.pos].trim().to_owned(),
                    ),
                    span: Span {
                        start,
                        end: self.pos,
                    },
                }));
            } else if self.starts_with(b"//") {
                let start = self.pos;
                self.pos += 2;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
                {
                    self.pos += 1;
                }
                lexemes.push(Lexeme::Trivia(Trivia {
                    kind: TriviaKind::LineComment,
                    span: Span {
                        start,
                        end: self.pos,
                    },
                }));
            } else if self.starts_with(b"/*") {
                let start = self.pos;
                self.pos += 2;
                while !self.starts_with(b"*/") {
                    if self.pos == self.bytes.len() {
                        return Err(Diagnostic::lexical(
                            "unterminated block comment",
                            Span {
                                start,
                                end: self.pos,
                            },
                        ));
                    }
                    self.pos += 1;
                }
                self.pos += 2;
                lexemes.push(Lexeme::Trivia(Trivia {
                    kind: TriviaKind::BlockComment,
                    span: Span {
                        start,
                        end: self.pos,
                    },
                }));
            } else {
                return Ok(None);
            }
        }
    }

    fn identifier(&mut self) -> TokenKind {
        let start = self.pos;
        self.pos += 1;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
        {
            self.pos += 1;
        }
        TokenKind::Ident(self.source[start..self.pos].to_owned())
    }

    fn number(&mut self) -> Result<TokenKind, Diagnostic> {
        let start = self.pos;
        let mut is_float = false;
        if self.starts_with(b"0x") || self.starts_with(b"0X") {
            self.pos += 2;
            let digits = self.pos;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_hexdigit() || *byte == b'_')
            {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(Diagnostic::lexical(
                    "expected hexadecimal digits after `0x`",
                    Span {
                        start,
                        end: self.pos,
                    },
                ));
            }
        } else {
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
            {
                self.pos += 1;
            }
            if self.bytes.get(self.pos) == Some(&b'.')
                && self.bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit)
            {
                is_float = true;
                self.pos += 1;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
                {
                    self.pos += 1;
                }
            }
        }

        if !is_float {
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            {
                self.pos += 1;
            }
        }
        let text = self.source[start..self.pos].to_owned();
        Ok(if is_float {
            TokenKind::Float(text)
        } else {
            TokenKind::Int(text)
        })
    }

    fn string(&mut self) -> Result<TokenKind, Diagnostic> {
        let start = self.pos;
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut value = String::new();
        while let Some(&byte) = self.bytes.get(self.pos) {
            if byte == quote {
                self.pos += 1;
                return Ok(TokenKind::String(value));
            }
            if byte == b'\n' || byte == b'\r' {
                break;
            }
            if byte == b'\\' {
                self.pos += 1;
                let Some(&escaped) = self.bytes.get(self.pos) else {
                    break;
                };
                value.push(match escaped {
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'\\' => '\\',
                    b'"' => '"',
                    b'\'' => '\'',
                    _ => {
                        return Err(Diagnostic::lexical(
                            "unsupported string escape",
                            Span {
                                start: self.pos - 1,
                                end: self.pos + 1,
                            },
                        ));
                    }
                });
                self.pos += 1;
            } else if byte.is_ascii() {
                value.push(byte as char);
                self.pos += 1;
            } else {
                let ch = self.source[self.pos..].chars().next().unwrap();
                value.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err(Diagnostic::lexical(
            "unterminated string literal",
            Span {
                start,
                end: self.pos,
            },
        ))
    }

    fn starts_with(&self, pattern: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(pattern)
    }

    fn one(&mut self, token: TokenKind) -> TokenKind {
        self.pos += 1;
        token
    }

    fn many(&mut self, count: usize, token: TokenKind) -> TokenKind {
        self.pos += count;
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_language_operators_and_typed_integer() {
        let tokens = lex(
            "current.level != old.level && 0xffu32 > 2; a += 1; b -= 1; c *= 2; d /= 2; e %= 2; f |= 1; g &= 1; h ^= 1; i <<= 1; j >>= 1",
        )
        .unwrap();
        assert!(tokens.iter().any(|token| token.kind == TokenKind::BangEq));
        assert!(tokens.iter().any(|token| token.kind == TokenKind::AndAnd));
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Int("0xffu32".into()))
        );
        for expected in [
            TokenKind::PlusAssign,
            TokenKind::MinusAssign,
            TokenKind::StarAssign,
            TokenKind::SlashAssign,
            TokenKind::PercentAssign,
            TokenKind::OrAssign,
            TokenKind::AndAssign,
            TokenKind::CaretAssign,
            TokenKind::ShlAssign,
            TokenKind::ShrAssign,
        ] {
            assert!(tokens.iter().any(|token| token.kind == expected));
        }
    }

    #[test]
    fn lexes_nested_template_interpolations_and_escapes() {
        let tokens = lex(r#"`Level {stage + 1}: {`act {act}`} \{literal\}`"#).unwrap();
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == TokenKind::TemplateStart)
                .count(),
            2
        );
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == TokenKind::TemplateExprStart)
                .count(),
            3
        );
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == TokenKind::TemplateExprEnd)
                .count(),
            3
        );
        assert!(tokens.iter().any(|token| {
            matches!(&token.kind, TokenKind::TemplateChunk(value) if value.contains("{literal}"))
        }));
    }

    #[test]
    fn preserves_doc_comments_but_discards_ordinary_comments() {
        let tokens = lex("/// First line\n/// second line\n// ordinary\nvalue").unwrap();
        assert_eq!(
            tokens
                .iter()
                .filter_map(|token| match &token.kind {
                    TokenKind::DocComment(value) => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["First line", "second line"]
        );
        assert!(
            tokens
                .iter()
                .any(|token| matches!(&token.kind, TokenKind::Ident(value) if value == "value"))
        );
    }
}
