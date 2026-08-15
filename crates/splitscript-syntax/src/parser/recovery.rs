//! Token consumption, diagnostics, and grammar recovery.

//! Syntax recovery policy and synchronization.

use super::{
    ActionKind, Diagnostic, Parser, RecoveryNode, RecoveryNodeKind, Span, Token, TokenKind,
    parse_integer,
};
use crate::migration::{
    ForeignSpellingContext, MigrationDiagnosticId, diagnostic as migration_diagnostic,
    foreign_spelling, legacy_lifecycle_diagnostic,
};

impl Parser<'_> {
    pub(super) fn terminator(&mut self) -> Result<(), Diagnostic> {
        if self.eat(&TokenKind::Semicolon).is_some()
            || self.at(&TokenKind::RBrace)
            || self.at(&TokenKind::Eof)
            || self.line_break_before_current()
        {
            Ok(())
        } else {
            Err(self.error("expected `;` or a line break after the statement"))
        }
    }

    pub(super) fn line_break_before_current(&self) -> bool {
        let previous_end = self.previous().span.end;
        let current_start = self.current().span.start;
        self.source[previous_end.min(self.source.len())..current_start.min(self.source.len())]
            .contains(['\n', '\r'])
    }

    pub(super) fn expect_u64(&mut self, message: &'static str) -> Result<u64, Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::Int(text) = &token.kind {
            let (value, suffix) =
                parse_integer(text).map_err(|error| Diagnostic::new(error, token.span))?;
            if suffix.is_some_and(|ty| !ty.is_integer()) {
                return Err(Diagnostic::new(message, token.span));
            }
            self.bump();
            Ok(value)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    pub(super) fn expect_i64(&mut self, message: &'static str) -> Result<i64, Diagnostic> {
        let minus = self.eat(&TokenKind::Minus);
        let token = self.current().clone();
        let TokenKind::Int(text) = &token.kind else {
            return Err(Diagnostic::new(message, minus.unwrap_or(token.span)));
        };
        let (magnitude, suffix) =
            parse_integer(text).map_err(|error| Diagnostic::new(error, token.span))?;
        if suffix.is_some_and(|ty| !ty.is_integer()) {
            return Err(Diagnostic::new(message, token.span));
        }
        let span = minus.map_or(token.span, |minus| minus.join(token.span));
        let value = if minus.is_some() {
            if magnitude == (i64::MAX as u64) + 1 {
                i64::MIN
            } else {
                -i64::try_from(magnitude).map_err(|_| {
                    Diagnostic::new("a pointer offset must fit in signed 64 bits", span)
                })?
            }
        } else {
            i64::try_from(magnitude)
                .map_err(|_| Diagnostic::new("a pointer offset must fit in signed 64 bits", span))?
        };
        self.bump();
        Ok(value)
    }

    pub(super) fn expect_bool(&mut self, message: &'static str) -> Result<bool, Diagnostic> {
        let token = self.current().clone();
        match &token.kind {
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                Ok(true)
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                Ok(false)
            }
            _ => Err(Diagnostic::new(message, token.span)),
        }
    }

    pub(super) fn expect_string(&mut self, message: &'static str) -> Result<String, Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::String(value) = token.kind {
            self.bump();
            Ok(value)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    pub(super) fn expect_ident(&mut self, expected: &'static str) -> Result<Span, Diagnostic> {
        let token = self.current().clone();
        if matches!(&token.kind, TokenKind::Ident(name) if name == expected) {
            self.bump();
            Ok(token.span)
        } else {
            Err(Diagnostic::new(
                format!("expected `{expected}`"),
                token.span,
            ))
        }
    }

    pub(super) fn expect_any_ident(
        &mut self,
        message: &'static str,
    ) -> Result<(String, Span), Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::Ident(name) = token.kind {
            self.bump();
            Ok((name, token.span))
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    pub(super) fn expect(
        &mut self,
        kind: TokenKind,
        message: &'static str,
    ) -> Result<Span, Diagnostic> {
        let token = self.current().clone();
        if token.kind == kind {
            self.bump();
            Ok(token.span)
        } else {
            Err(Diagnostic::new(message, token.span))
        }
    }

    pub(super) fn expect_generic_close(
        &mut self,
        message: &'static str,
    ) -> Result<Span, Diagnostic> {
        self.eat_generic_close()
            .ok_or_else(|| Diagnostic::new(message, self.current().span))
    }

    pub(super) fn eat_generic_close(&mut self) -> Option<Span> {
        self.cursor.eat_leading_gt()
    }

    pub(super) fn eat_fallible_type_suffix(&mut self) -> Option<Span> {
        self.cursor.eat_leading_bang()
    }

    pub(super) fn at_generic_close(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Gt | TokenKind::Ge | TokenKind::Shr | TokenKind::ShrAssign
        )
    }

    pub(super) fn eat_ident(&mut self, expected: &str) -> Option<Span> {
        if self.at_ident(expected) {
            Some(self.bump().span)
        } else {
            None
        }
    }

    pub(super) fn at_ident(&self, expected: &str) -> bool {
        self.cursor.at_ident(expected)
    }

    pub(super) fn eat(&mut self, kind: &TokenKind) -> Option<Span> {
        if self.at(kind) {
            Some(self.bump().span)
        } else {
            None
        }
    }

    pub(super) fn at(&self, kind: &TokenKind) -> bool {
        self.cursor.at(kind)
    }

    pub(super) fn current(&self) -> &Token {
        self.cursor.current()
    }

    pub(super) fn previous(&self) -> &Token {
        self.cursor.previous()
    }

    pub(super) fn peek(&self, offset: usize) -> &Token {
        self.cursor.peek(offset)
    }

    pub(super) fn bump(&mut self) -> &Token {
        self.cursor.bump()
    }

    pub(super) fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(message, self.current().span)
    }

    pub(super) fn record_let_keyword_diagnostic(&mut self) {
        let span = self.current().span;
        let TokenKind::Ident(keyword) = &self.current().kind else {
            unreachable!("the familiar declaration keyword is an identifier")
        };
        let keyword = keyword.clone();
        self.record_foreign_spelling_diagnostic(
            span,
            &keyword,
            ForeignSpellingContext::VariableDeclaration,
        );
    }

    pub(super) fn record_fn_keyword_diagnostic(&mut self) {
        let span = self.current().span;
        let TokenKind::Ident(keyword) = &self.current().kind else {
            unreachable!("the familiar function keyword is an identifier")
        };
        let keyword = keyword.clone();
        self.record_foreign_spelling_diagnostic(
            span,
            &keyword,
            ForeignSpellingContext::FunctionDeclaration,
        );
    }

    pub(super) fn record_foreign_spelling_diagnostic(
        &mut self,
        span: Span,
        spelling: &str,
        context: ForeignSpellingContext,
    ) -> Option<&'static str> {
        let rule = foreign_spelling(spelling, context)?;
        let replacement = rule.replacement.text();
        self.diagnostics.push(
            Diagnostic::new(rule.message, span)
                .with_primary_label(rule.primary_label)
                .with_machine_applicable_fix(rule.fix_title, span, replacement),
        );
        Some(replacement)
    }

    pub(super) fn migration_diagnostic(&self, id: MigrationDiagnosticId, span: Span) -> Diagnostic {
        let metadata = migration_diagnostic(id)
            .expect("parser migration diagnostic IDs must exist in the migration catalog");
        let mut diagnostic =
            Diagnostic::new(metadata.message, span).with_primary_label(metadata.primary_label);
        for note in metadata.notes {
            diagnostic = diagnostic.with_note(*note);
        }
        diagnostic
    }

    pub(super) fn current_action_kind(&self) -> Option<ActionKind> {
        let TokenKind::Ident(name) = &self.current().kind else {
            return None;
        };
        ActionKind::parse(name)
    }

    pub(super) fn current_legacy_lifecycle_diagnostic(&self) -> Option<MigrationDiagnosticId> {
        let TokenKind::Ident(name) = &self.current().kind else {
            return None;
        };
        legacy_lifecycle_diagnostic(name)
    }

    pub(super) fn is_top_level_start(&self) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::Ident(name)
                if matches!(name.as_str(), "state" | "tickRate" | "settings" | "let" | "const" | "var" | "debug" | "fn" | "func" | "function" | "record" | "enum")
                    || ActionKind::parse(name).is_some()
                    || legacy_lifecycle_diagnostic(name).is_some()
        )
    }

    pub(super) fn synchronize_top_level(&mut self, declaration_start: usize) {
        let mut brace_depth = self.brace_depth_before(self.cursor.position());

        if self.cursor.position() > declaration_start
            && brace_depth == 0
            && self.is_top_level_start()
        {
            return;
        }

        while !self.at(&TokenKind::Eof) {
            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
            if brace_depth == 0 && self.is_top_level_start() {
                return;
            }
        }
    }

    pub(super) fn synchronize_statement(&mut self, statement_start: usize, block_depth: u32) {
        let mut brace_depth = self.brace_depth_before(self.cursor.position());
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::RBrace) && brace_depth == block_depth)
            {
                return;
            }
            if self.cursor.position() > statement_start
                && brace_depth == block_depth
                && self.line_break_before_current()
                && self.is_statement_start()
            {
                return;
            }

            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::Semicolon if brace_depth == block_depth => return,
                _ => {}
            }
        }
    }

    pub(super) fn recover_delimited_item<T>(
        &mut self,
        parsed: Result<T, Diagnostic>,
        item_start: usize,
        body_depth: u32,
    ) -> Option<T> {
        match parsed {
            Ok(item) => Some(item),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[item_start].span.start;
                self.synchronize_delimited_item(item_start, body_depth);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    pub(super) fn recover_parameter<T>(
        &mut self,
        parsed: Result<T, Diagnostic>,
        item_start: usize,
    ) -> Option<T> {
        match parsed {
            Ok(parameter) => Some(parameter),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[item_start].span.start;
                self.synchronize_parameter(item_start);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    pub(super) fn record_recovery_diagnostic(&mut self, error: Diagnostic) {
        if error.message.starts_with("expected") {
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Missing,
                span: Span {
                    start: error.span.start,
                    end: error.span.start,
                },
            });
        }
        self.diagnostics.push(error);
    }

    pub(super) fn record_error_region(&mut self, start: usize, end: usize) {
        let end = end.max(start);
        if end != start {
            self.recovery_nodes.push(RecoveryNode {
                kind: RecoveryNodeKind::Error,
                span: Span { start, end },
            });
        }
    }

    pub(super) fn synchronize_parameter(&mut self, item_start: usize) {
        loop {
            if self.at(&TokenKind::Eof)
                || self.at(&TokenKind::RParen)
                || self.at(&TokenKind::LBrace)
                || self.at(&TokenKind::Minus)
            {
                return;
            }
            if self.cursor.position() > item_start
                && self.line_break_before_current()
                && matches!(self.current().kind, TokenKind::Ident(_))
            {
                return;
            }
            if matches!(self.bump().kind, TokenKind::Comma) {
                return;
            }
        }
    }

    pub(super) fn synchronize_delimited_item(&mut self, item_start: usize, body_depth: u32) {
        let mut brace_depth = self.brace_depth_before(self.cursor.position());
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::RBrace) && brace_depth == body_depth)
            {
                return;
            }
            if self.cursor.position() > item_start
                && brace_depth == body_depth
                && self.line_break_before_current()
                && matches!(
                    self.current().kind,
                    TokenKind::Ident(_)
                        | TokenKind::Int(_)
                        | TokenKind::Char(_)
                        | TokenKind::String(_)
                        | TokenKind::DocComment(_)
                )
            {
                return;
            }

            let kind = self.bump().kind.clone();
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::Comma | TokenKind::Semicolon if brace_depth == body_depth => return,
                _ => {}
            }
        }
    }

    pub(super) fn record_missing_closing(&mut self, message: &'static str) {
        let error = self.error(message);
        self.record_missing(error);
    }

    pub(super) fn record_missing(&mut self, error: Diagnostic) {
        let position = error.span.start;
        self.diagnostics.push(error);
        self.recovery_nodes.push(RecoveryNode {
            kind: RecoveryNodeKind::Missing,
            span: Span {
                start: position,
                end: position,
            },
        });
    }

    pub(super) fn brace_depth_before(&self, position: usize) -> u32 {
        self.cursor.tokens()[..position]
            .iter()
            .fold(0u32, |depth, token| match token.kind {
                TokenKind::LBrace => depth + 1,
                TokenKind::RBrace => depth.saturating_sub(1),
                _ => depth,
            })
    }

    pub(super) fn is_statement_start(&self) -> bool {
        match &self.current().kind {
            TokenKind::Ident(name) => name != "else",
            _ => self.is_expression_start(),
        }
    }

    pub(super) fn is_expression_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Ident(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Char(_)
                | TokenKind::String(_)
                | TokenKind::TemplateStart
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::Bang
                | TokenKind::Minus
        )
    }
}
