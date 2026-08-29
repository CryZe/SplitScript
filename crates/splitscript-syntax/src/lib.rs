//! Shared, dependency-light SplitScript syntax infrastructure.
//!
//! Both ordinary programs and the privileged bundled standard library use the
//! same spans, tokens, string escapes, comments, and lexical rules. Syntax
//! mode controls only constructs that are deliberately unavailable to user
//! programs.

pub mod ast;
mod cursor;
pub mod diagnostic;
mod lexer;
pub mod migration;
pub mod parser;
pub mod source;
pub mod standard_library;
pub mod visit;

use std::fmt;

pub use cursor::TokenCursor;
pub use lexer::{Lexed, Lexeme, Token, TokenKind, Trivia, TriviaKind, lex, lex_lossless};

/// Returns whether `byte` can begin a SplitScript identifier.
///
/// Keep cursor-oriented tooling on this syntax-owned definition instead of
/// recreating a slightly different language grammar in each consumer.
pub const fn is_identifier_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// Returns whether `byte` can continue a SplitScript identifier.
pub const fn is_identifier_continue_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Returns whether `name` is one complete SplitScript identifier.
pub fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(is_identifier_start_byte) && bytes.all(is_identifier_continue_byte)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxMode {
    Program,
    StandardLibrary,
}

/// Primitive type spellings understood directly by the language grammar.
///
/// These are the only types whose identity is intrinsically syntactic. Every
/// nominal standard-library or user-defined type remains a name until the
/// resolver assigns it a semantic identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrimitiveType {
    Never,
    None,
    Bool,
    Char,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Address,
    F32,
    F64,
}

impl PrimitiveType {
    pub const ALL: &'static [Self] = &[
        Self::Never,
        Self::None,
        Self::Bool,
        Self::Char,
        Self::I8,
        Self::U8,
        Self::I16,
        Self::U16,
        Self::I32,
        Self::U32,
        Self::I64,
        Self::U64,
        Self::Address,
        Self::F32,
        Self::F64,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::None => "None",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Address => "address",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        if name == "Address" {
            return Some(Self::Address);
        }
        Self::ALL.iter().copied().find(|ty| ty.name() == name)
    }

    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::Address
        )
    }

    pub const fn is_numeric(self) -> bool {
        self.is_integer() || matches!(self, Self::F32 | Self::F64)
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub span: Span,
}

impl Error {
    fn lexical(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::{PrimitiveType, is_identifier};

    #[test]
    fn primitive_spellings_round_trip_without_catalog_state() {
        for ty in PrimitiveType::ALL {
            assert_eq!(PrimitiveType::parse(ty.name()), Some(*ty));
        }
        assert_eq!(
            PrimitiveType::parse("Address"),
            Some(PrimitiveType::Address)
        );
        assert_eq!(PrimitiveType::parse("String"), None);
    }

    #[test]
    fn identifiers_use_the_language_grammar() {
        for valid in ["value", "_value", "value2"] {
            assert!(is_identifier(valid), "`{valid}` should be an identifier");
        }
        for invalid in ["", "2value", "$value", "value$", "value-name"] {
            assert!(
                !is_identifier(invalid),
                "`{invalid}` should not be an identifier"
            );
        }
    }
}
