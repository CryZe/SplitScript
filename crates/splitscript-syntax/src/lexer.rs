use crate::{Error, Span, SyntaxMode};

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
    Char(char),
    String(String),
    DocComment(String),
    At,
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
    DotDot,
    DotDotLt,
    DotDotEq,
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

pub fn lex(source: &str, syntax_mode: SyntaxMode) -> Result<Vec<Token>, Error> {
    Ok(lex_lossless(source, syntax_mode)?
        .tokens()
        .cloned()
        .collect())
}

pub fn lex_lossless(source: &str, syntax_mode: SyntaxMode) -> Result<Lexed, Error> {
    Lexer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        modes: vec![LexMode::Code],
        syntax_mode,
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
    syntax_mode: SyntaxMode,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Lexed, Error> {
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
                    return Err(Error::lexical(
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
                b'"' => self.string()?,
                b'\'' => self.character()?,
                b'`' => {
                    self.pos += 1;
                    self.modes.push(LexMode::Template { start });
                    TokenKind::TemplateStart
                }
                b'@' if self.syntax_mode == SyntaxMode::StandardLibrary => self.one(TokenKind::At),
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
                b'.' if self.starts_with(b"..<") => self.many(3, TokenKind::DotDotLt),
                b'.' if self.starts_with(b"..=") => self.many(3, TokenKind::DotDotEq),
                b'.' if self.starts_with(b"..") => self.many(2, TokenKind::DotDot),
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
                b'=' if self.starts_with(b"===") => self.many(3, TokenKind::EqEq),
                b'=' if self.starts_with(b"==") => self.many(2, TokenKind::EqEq),
                b'=' => self.one(TokenKind::Assign),
                b'!' if self.starts_with(b"!==") => self.many(3, TokenKind::BangEq),
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
                    return Err(Error::lexical("unexpected character", Span { start, end }));
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

    fn template_token(&mut self) -> Result<Token, Error> {
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
                return Err(Error::lexical(
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
                        return Err(Error::lexical(
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
        Err(Error::lexical(
            "unterminated template literal",
            Span {
                start: template_start,
                end: self.pos,
            },
        ))
    }

    fn skip_trivia(&mut self, lexemes: &mut Vec<Lexeme>) -> Result<Option<Token>, Error> {
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
                let content = &self.source[content_start..self.pos];
                let content = content.strip_prefix(' ').unwrap_or(content).trim_end();
                return Ok(Some(Token {
                    kind: TokenKind::DocComment(content.to_owned()),
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
                        return Err(Error::lexical(
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

    fn number(&mut self) -> Result<TokenKind, Error> {
        let start = self.pos;
        let mut is_float = false;
        if self.starts_with(b"0x") || self.starts_with(b"0X") {
            self.pos += 2;
            let mut has_digit = false;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_hexdigit() || *byte == b'_')
            {
                has_digit |= self.bytes[self.pos].is_ascii_hexdigit();
                self.pos += 1;
            }
            if !has_digit {
                return Err(Error::lexical(
                    "expected hexadecimal digits after `0x`",
                    Span {
                        start,
                        end: self.pos,
                    },
                ));
            }
        } else if self.starts_with(b"0b") || self.starts_with(b"0B") {
            self.pos += 2;
            let mut has_digit = false;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| matches!(byte, b'0' | b'1' | b'_'))
            {
                has_digit |= matches!(self.bytes[self.pos], b'0' | b'1');
                self.pos += 1;
            }
            if !has_digit {
                return Err(Error::lexical(
                    "expected binary digits after `0b`",
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
            if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
                is_float = true;
                self.pos += 1;
                if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                let exponent_digits = self.pos;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
                {
                    self.pos += 1;
                }
                if self.pos == exponent_digits {
                    return Err(Error::lexical(
                        "expected exponent digits after `e`",
                        Span {
                            start,
                            end: self.pos,
                        },
                    ));
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

    fn string(&mut self) -> Result<TokenKind, Error> {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        while let Some(&byte) = self.bytes.get(self.pos) {
            if byte == b'"' {
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
                        return Err(Error::lexical(
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
        Err(Error::lexical(
            "unterminated string literal",
            Span {
                start,
                end: self.pos,
            },
        ))
    }

    fn character(&mut self) -> Result<TokenKind, Error> {
        let start = self.pos;
        self.pos += 1;
        let value_start = self.pos;
        let Some(&byte) = self.bytes.get(self.pos) else {
            return Err(Error::lexical(
                "unterminated character literal",
                Span {
                    start,
                    end: self.pos,
                },
            ));
        };
        if byte == b'\'' {
            self.pos += 1;
            return Err(Error::lexical(
                "a character literal must contain exactly one Unicode scalar value",
                Span {
                    start,
                    end: self.pos,
                },
            ));
        }
        if matches!(byte, b'\r' | b'\n') {
            return Err(Error::lexical(
                "unterminated character literal",
                Span {
                    start,
                    end: self.pos,
                },
            ));
        }

        let value = if byte == b'\\' {
            self.character_escape(start)?
        } else {
            let value = self.source[self.pos..]
                .chars()
                .next()
                .expect("the current byte begins a source character");
            self.pos += value.len_utf8();
            value
        };

        if self.bytes.get(self.pos) != Some(&b'\'') {
            while let Some(&byte) = self.bytes.get(self.pos) {
                if matches!(byte, b'\r' | b'\n') {
                    break;
                }
                self.pos += 1;
                if byte == b'\'' {
                    break;
                }
            }
            return Err(Error::lexical(
                "a character literal must contain exactly one Unicode scalar value",
                Span {
                    start: value_start,
                    end: self.pos,
                },
            ));
        }
        self.pos += 1;
        Ok(TokenKind::Char(value))
    }

    fn character_escape(&mut self, literal_start: usize) -> Result<char, Error> {
        let escape_start = self.pos;
        self.pos += 1;
        let Some(&escaped) = self.bytes.get(self.pos) else {
            return Err(Error::lexical(
                "unterminated character literal",
                Span {
                    start: literal_start,
                    end: self.pos,
                },
            ));
        };
        let value = match escaped {
            b'0' => '\0',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'\\' => '\\',
            b'"' => '"',
            b'\'' => '\'',
            b'u' => return self.unicode_character_escape(escape_start),
            _ => {
                self.pos += 1;
                return Err(Error::lexical(
                    "unsupported character escape",
                    Span {
                        start: escape_start,
                        end: self.pos,
                    },
                ));
            }
        };
        self.pos += 1;
        Ok(value)
    }

    fn unicode_character_escape(&mut self, escape_start: usize) -> Result<char, Error> {
        self.pos += 1;
        if self.bytes.get(self.pos) != Some(&b'{') {
            return Err(Error::lexical(
                "a Unicode character escape must use `\\u{...}`",
                Span {
                    start: escape_start,
                    end: self.pos,
                },
            ));
        }
        self.pos += 1;
        let digits_start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            self.pos += 1;
        }
        let digit_count = self.pos - digits_start;
        if digit_count == 0 || digit_count > 6 || self.bytes.get(self.pos) != Some(&b'}') {
            return Err(Error::lexical(
                "a Unicode character escape requires one to six hexadecimal digits",
                Span {
                    start: escape_start,
                    end: self.pos,
                },
            ));
        }
        let value = u32::from_str_radix(&self.source[digits_start..self.pos], 16)
            .expect("validated hexadecimal digits fit in u32");
        self.pos += 1;
        char::from_u32(value).ok_or_else(|| {
            Error::lexical(
                "a character literal must be a Unicode scalar value",
                Span {
                    start: escape_start,
                    end: self.pos,
                },
            )
        })
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
            "current.level != old.level && 0xffu32 > 0b10; let mask = 0B1111_0000u8; a += 1; b -= 1; c *= 2; d /= 2; e %= 2; f |= 1; g &= 1; h ^= 1; i <<= 1; j >>= 1",
            SyntaxMode::Program,
        )
        .unwrap();
        assert!(tokens.iter().any(|token| token.kind == TokenKind::BangEq));
        assert!(tokens.iter().any(|token| token.kind == TokenKind::AndAnd));
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Int("0xffu32".into()))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Int("0b10".into()))
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Int("0B1111_0000u8".into()))
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
    fn rejects_binary_and_hexadecimal_prefixes_without_digits() {
        for (source, message) in [
            ("0b", "expected binary digits after `0b`"),
            ("0B_", "expected binary digits after `0b`"),
            ("0x", "expected hexadecimal digits after `0x`"),
            ("0X_", "expected hexadecimal digits after `0x`"),
        ] {
            let error = lex(source, SyntaxMode::Program)
                .expect_err("a radix prefix requires at least one digit");
            assert_eq!(error.message, message);
        }
    }

    #[test]
    fn lexes_nested_template_interpolations_and_escapes() {
        let tokens = lex(
            r#"`Level {stage + 1}: {`act {act}`} \{literal\}`"#,
            SyntaxMode::Program,
        )
        .unwrap();
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
    fn lexes_decimal_exponents_as_floating_point_literals() {
        let tokens = lex("1e-45 5E-324 6.022e+23", SyntaxMode::Program).unwrap();
        assert_eq!(
            tokens
                .iter()
                .filter_map(|token| match &token.kind {
                    TokenKind::Float(value) => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["1e-45", "5E-324", "6.022e+23"]
        );

        for source in ["1e", "1e+", "1.0E-"] {
            let error = lex(source, SyntaxMode::Program)
                .expect_err("an exponent requires at least one digit");
            assert_eq!(error.message, "expected exponent digits after `e`");
        }
    }

    #[test]
    fn lexes_exactly_one_unicode_scalar_per_character_literal() {
        let source = format!(
            "'a' '{}' '{}' '\\n' '\\'' '\\u{{1f642}}'",
            '\u{df}', '\u{1f642}'
        );
        let tokens = lex(&source, SyntaxMode::Program).unwrap();
        assert_eq!(
            tokens
                .iter()
                .filter_map(|token| match token.kind {
                    TokenKind::Char(value) => Some(value),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ['a', '\u{df}', '\u{1f642}', '\n', '\'', '\u{1f642}']
        );

        for source in ["''", "'ab'", "'e\u{301}'"] {
            let error = lex(source, SyntaxMode::Program)
                .expect_err("character literals contain exactly one Unicode scalar");
            assert!(error.message.contains("exactly one Unicode scalar value"));
        }
    }

    #[test]
    fn rejects_non_scalar_unicode_character_escapes() {
        for source in [r"'\u{d800}'", r"'\u{110000}'"] {
            let error = lex(source, SyntaxMode::Program)
                .expect_err("surrogates and out-of-range values are not Unicode scalars");
            assert!(error.message.contains("Unicode scalar value"));
        }
    }

    #[test]
    fn preserves_doc_comments_but_discards_ordinary_comments() {
        let tokens = lex(
            "/// First line\n/// second line\n// ordinary\nvalue",
            SyntaxMode::Program,
        )
        .unwrap();
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

    #[test]
    fn doc_comments_remove_only_the_conventional_single_leading_space() {
        let tokens = lex(
            "/// ```splitscript\n/// if true {\n///     print(\"nested\")\n/// }\nvalue",
            SyntaxMode::Program,
        )
        .unwrap();
        assert_eq!(
            tokens
                .iter()
                .filter_map(|token| match &token.kind {
                    TokenKind::DocComment(value) => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["```splitscript", "if true {", "    print(\"nested\")", "}"]
        );
    }

    #[test]
    fn privileged_punctuation_requires_standard_library_mode() {
        let source = "@intrinsic(Print) fn print() -> None;";
        let error = lex(source, SyntaxMode::Program).unwrap_err();
        assert_eq!(error.span, Span { start: 0, end: 1 });

        let tokens = lex(source, SyntaxMode::StandardLibrary).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::At);
    }

    #[test]
    fn common_syntax_has_identical_tokens_in_both_modes() {
        let source = "/// Documentation\nfn value(input: [u32]?) -> String!;";
        assert_eq!(
            lex(source, SyntaxMode::Program).unwrap(),
            lex(source, SyntaxMode::StandardLibrary).unwrap()
        );
    }
}
