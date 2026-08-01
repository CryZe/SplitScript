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

fn into_diagnostic(error: splitscript_syntax::Error) -> Diagnostic {
    Diagnostic::lexical(error.message, error.span)
}
