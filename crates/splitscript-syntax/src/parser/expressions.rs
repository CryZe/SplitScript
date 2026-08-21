//! Expressions, patterns, precedence, and expression-local recovery.

//! Expression grammar.

use super::{
    BinaryOp, DelimiterDepth, Diagnostic, EnumReference, Expr, ExprKind, InterpolatedPart,
    MatchArm, MatchPattern, Parser, PatternBinding, Span, TokenKind, TypeRef, UnaryOp,
    assignment_operator, parse_integer,
};
use crate::diagnostic::DiagnosticCode;

impl Parser<'_> {
    pub(super) fn expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let mut left = self.prefix()?;
        let mut saw_comparison = false;
        loop {
            if self.line_break_before_current()
                && matches!(
                    &left.kind,
                    ExprKind::Suspend { value, .. } if matches!(value.kind, ExprKind::Error)
                )
            {
                // A recovered suspension operand owns only its source line.
                // Do not reinterpret an operator starting the next malformed
                // statement as a continuation of this expression.
                break;
            }
            if (self.at(&TokenKind::LParen) || self.begins_generic_call())
                && matches!(&left.kind, ExprKind::Member { .. })
            {
                let start = left.span;
                let ExprKind::Member {
                    receiver,
                    name,
                    name_span,
                } = left.kind
                else {
                    unreachable!()
                };
                let mut callee = Vec::new();
                let receiver = flatten_postfix_receiver(*receiver, &mut callee);
                callee.push(name);
                let (type_arguments, type_argument_span) = self.call_type_arguments()?;
                self.expect(TokenKind::LParen, "expected `(` after generic arguments")?;
                let (args, end) =
                    self.expression_list(TokenKind::RParen, "expected `)` after arguments", true);
                left = self.new_expr(
                    ExprKind::Call {
                        callee,
                        name_span,
                        receiver: Some(Box::new(receiver)),
                        type_arguments,
                        type_argument_span,
                        args,
                    },
                    start.join(end),
                );
                continue;
            }
            if self.eat(&TokenKind::Dot).is_some() {
                let (name, name_span) = self.expect_any_ident("expected a field name after `.`")?;
                let span = left.span.join(name_span);
                left = self.new_expr(
                    ExprKind::Member {
                        receiver: Box::new(left),
                        name,
                        name_span,
                    },
                    span,
                );
                continue;
            }
            if let Some(opening) = self.eat(&TokenKind::LBracket) {
                let index = self.required_expression(0)?;
                let closing = self.expect(TokenKind::RBracket, "expected `]` after the index")?;
                let span = left.span.join(closing);
                left = self.new_expr(
                    ExprKind::Index {
                        receiver: Box::new(left),
                        index: Box::new(index),
                        bracket_span: opening.join(closing),
                    },
                    span,
                );
                continue;
            }
            if let Some(question) = self.eat(&TokenKind::Question) {
                let span = left.span.join(question);
                left = self.new_expr(ExprKind::Propagate(Box::new(left)), span);
                continue;
            }
            const FALLBACK_PRECEDENCE: u8 = 0;
            if self.at_ident("else") {
                if FALLBACK_PRECEDENCE < min_precedence {
                    break;
                }
                self.bump();
                let fallback = self.required_expression(FALLBACK_PRECEDENCE)?;
                let span = left.span.join(fallback.span);
                left = self.new_expr(
                    ExprKind::Fallback {
                        value: Box::new(left),
                        fallback: Box::new(fallback),
                    },
                    span,
                );
                continue;
            }
            const CAST_PRECEDENCE: u8 = 10;
            if self.at_ident("as") {
                if CAST_PRECEDENCE < min_precedence {
                    break;
                }
                self.bump();
                let (target, target_span) = self.parse_type("expected a type after `as`")?;
                let span = left.span.join(target_span);
                left = self.new_expr(
                    ExprKind::Cast {
                        expr: Box::new(left),
                        target,
                    },
                    span,
                );
                continue;
            }
            let Some((precedence, op)) = self.binary_operator() else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            let operator_span = self.current().span;
            if let Some(spelling) = self.source.get(operator_span.start..operator_span.end)
                && matches!(spelling, "===" | "!==")
            {
                self.record_foreign_spelling_diagnostic(
                    operator_span,
                    spelling,
                    crate::migration::ForeignSpellingContext::Operator,
                );
            }
            let is_comparison = matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            );
            if is_comparison && saw_comparison {
                return Err(self.error(
                    "comparison operators cannot be chained; use parentheses to disambiguate",
                ));
            }
            self.bump();
            let right = self.required_expression(precedence + 1)?;
            let span = left.span.join(right.span);
            left = self.new_expr(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            );
            saw_comparison |= is_comparison;
        }
        Ok(left)
    }

    pub(super) fn prefix(&mut self) -> Result<Expr, Diagnostic> {
        if self.eat_ident("return").is_some() {
            let start = self.previous().span;
            let value = self.optional_control_flow_value()?;
            let span = value.as_ref().map_or(start, |value| start.join(value.span));
            return Ok(self.new_expr(ExprKind::Return(value.map(Box::new)), span));
        }
        if self.eat_ident("break").is_some() {
            let start = self.previous().span;
            let value = self.optional_control_flow_value()?;
            let span = value.as_ref().map_or(start, |value| start.join(value.span));
            return Ok(self.new_expr(ExprKind::Break(value.map(Box::new)), span));
        }
        if self.eat_ident("continue").is_some() {
            let span = self.previous().span;
            return Ok(self.new_expr(ExprKind::Continue, span));
        }
        if self.eat_ident("throw").is_some() {
            let start = self.previous().span;
            let error = self.required_expression(0)?;
            let span = start.join(error.span);
            return Ok(self.new_expr(ExprKind::Throw(Box::new(error)), span));
        }
        if self.at_ident("await") || self.at_ident("retry") {
            let mode = if self.eat_ident("await").is_some() {
                super::SuspensionMode::Await
            } else {
                self.expect_ident("retry")?;
                super::SuspensionMode::Retry
            };
            let start = self.previous().span;
            let expression_start = self.cursor.position();
            let parsed = if self.expression_is_missing_before_statement() {
                Err(self.error("expected an expression"))
            } else {
                self.expression(11)
            };
            let value = self.recover_root_expression(parsed, expression_start);
            let span = start.join(value.span);
            let destination = self.new_value_id();
            return Ok(self.new_expr(
                ExprKind::Suspend {
                    mode,
                    destination,
                    value: Box::new(value),
                },
                span,
            ));
        }
        if self.eat_ident("if").is_some() {
            let start = self.previous().span;
            return self.if_expression(start);
        }
        if self.eat_ident("match").is_some() {
            let start = self.previous().span;
            let value = self.required_expression(0)?;
            self.expect(TokenKind::LBrace, "expected `{` after the matched value")?;
            let body_depth = self.brace_depth_before(self.cursor.position());
            let mut arms = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                if self.at(&TokenKind::Eof) {
                    self.record_missing_closing("unterminated match expression");
                    break;
                }
                let item_start = self.cursor.position();
                let parsed = self.match_arm();
                if let Some(arm) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    arms.push(arm);
                }
            }
            let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
            return Ok(self.new_expr(
                ExprKind::Match {
                    value: Box::new(value),
                    arms,
                },
                start.join(end),
            ));
        }
        if self.eat_ident("loop").is_some() {
            let start = self.previous().span;
            let block = self.block()?;
            let span = start.join(block.span);
            return Ok(self.new_expr(ExprKind::Loop(block), span));
        }
        if self.at(&TokenKind::LBrace) {
            let block = self.block()?;
            return Ok(self.value_block(block));
        }
        if self.eat(&TokenKind::Bang).is_some() {
            let start = self.previous().span;
            let expr = self.required_expression(11)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.eat(&TokenKind::Minus).is_some() {
            let start = self.previous().span;
            let expr = self.required_expression(11)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.at(&TokenKind::Tilde) {
            let start = self.current().span;
            self.record_foreign_spelling_diagnostic(
                start,
                "~",
                crate::migration::ForeignSpellingContext::Operator,
            );
            self.bump();
            let expr = self.required_expression(11)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.eat(&TokenKind::LParen).is_some() {
            let start = self.previous().span;
            let target_depth = self.delimiter_depth_before(self.cursor.position());
            let mut expr = self.required_expression(0)?;
            let end = if let Some(end) = self.eat(&TokenKind::RParen) {
                end
            } else {
                let error = self.error("expected `)` after expression");
                self.record_recovery_diagnostic(error);
                let skipped_start = self.current().span.start;
                self.synchronize_delimited_expression(&TokenKind::RParen, target_depth);
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::RParen).unwrap_or(expr.span)
            };
            expr.span = start.join(end);
            return Ok(expr);
        }
        if self.eat(&TokenKind::LBracket).is_some() {
            let start = self.previous().span;
            let (elements, end) = self.expression_list(
                TokenKind::RBracket,
                "expected `]` after array elements",
                true,
            );
            return Ok(self.new_expr(ExprKind::Array(elements), start.join(end)));
        }
        if self.eat(&TokenKind::TemplateStart).is_some() {
            let start = self.previous().span;
            let mut parts = Vec::new();
            let mut has_expression = false;
            loop {
                match self.current().kind.clone() {
                    TokenKind::TemplateChunk(value) => {
                        self.bump();
                        parts.push(InterpolatedPart::Text(value));
                    }
                    TokenKind::TemplateExprStart => {
                        self.bump();
                        let expression_start = self.cursor.position();
                        if let Some(value) = self.interpolated_expression(expression_start) {
                            has_expression = true;
                            parts.push(InterpolatedPart::Expr(value));
                        }
                    }
                    TokenKind::TemplateEnd => {
                        let end = self.bump().span;
                        if has_expression {
                            return Ok(
                                self.new_expr(ExprKind::InterpolatedString(parts), start.join(end))
                            );
                        }
                        let value = parts
                            .into_iter()
                            .map(|part| match part {
                                InterpolatedPart::Text(value) => value,
                                InterpolatedPart::Expr(_) => unreachable!(),
                            })
                            .collect();
                        return Ok(self.new_expr(ExprKind::String(value), start.join(end)));
                    }
                    _ => {
                        return Err(self.error(
                            "expected template text, an interpolation, or a closing backtick",
                        ));
                    }
                }
            }
        }

        let token = self.current().clone();
        match token.kind {
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_foreign_spelling_diagnostic(
                    token.span,
                    "null",
                    crate::migration::ForeignSpellingContext::OptionalValue,
                );
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(true), token.span))
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(false), token.span))
            }
            TokenKind::Ident(mut first) => {
                self.bump();
                if self.at(&TokenKind::Dot)
                    && let Some(replacement) = self.record_foreign_spelling_diagnostic(
                        token.span,
                        &first,
                        crate::migration::ForeignSpellingContext::StaticTypeReceiver,
                    )
                {
                    first = replacement.to_owned();
                }
                if first == "sig" {
                    let signature_span = self.current().span;
                    let value = self.expect_string("expected a quoted pattern after `sig`")?;
                    return Ok(
                        self.new_expr(ExprKind::Signature(value), token.span.join(signature_span))
                    );
                }
                if first == "v" {
                    let version_span = self.current().span;
                    let value = self.expect_string("expected a quoted version after `v`")?;
                    let components = parse_file_version(&value)
                        .map_err(|message| Diagnostic::new(message, version_span))?;
                    let args = components
                        .into_iter()
                        .map(|value| {
                            self.new_expr(
                                ExprKind::Int {
                                    value: u64::from(value),
                                    suffix: None,
                                },
                                version_span,
                            )
                        })
                        .collect();
                    return Ok(self.new_expr(
                        ExprKind::Call {
                            callee: vec!["FileVersion".to_owned(), "fromParts".to_owned()],
                            // The source spelling is a literal, not a visible
                            // call site. Keep the lowering target out of
                            // position-based tooling while preserving the
                            // complete literal span on the expression.
                            name_span: Span {
                                start: token.span.end,
                                end: token.span.end,
                            },
                            receiver: None,
                            type_arguments: Vec::new(),
                            type_argument_span: None,
                            args,
                        },
                        token.span.join(version_span),
                    ));
                }
                let begins_record_literal = self.at(&TokenKind::LBrace)
                    && (matches!(self.peek(1).kind, TokenKind::RBrace)
                        || matches!(
                            (&self.peek(1).kind, &self.peek(2).kind),
                            (TokenKind::Ident(_), TokenKind::Colon)
                        ));
                if begins_record_literal && self.eat(&TokenKind::LBrace).is_some() {
                    let body_depth = self.brace_depth_before(self.cursor.position());
                    let mut fields = Vec::new();
                    while !self.at(&TokenKind::RBrace) {
                        if self.at(&TokenKind::Eof) {
                            self.record_missing_closing("unterminated record literal");
                            break;
                        }
                        let item_start = self.cursor.position();
                        let parsed = self.record_literal_field();
                        if let Some(field) =
                            self.recover_delimited_item(parsed, item_start, body_depth)
                        {
                            fields.push(field);
                            if self.eat(&TokenKind::Comma).is_some() {
                                continue;
                            }
                            if self.at(&TokenKind::RBrace) {
                                continue;
                            }
                            self.record_missing(Diagnostic::new(
                                "expected `,` between record fields",
                                self.current().span,
                            ));
                            if matches!(self.current().kind, TokenKind::Ident(_)) {
                                continue;
                            }
                            self.synchronize_delimited_item(item_start, body_depth);
                        }
                    }
                    let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
                    return Ok(self.new_expr(
                        ExprKind::Record {
                            name: first,
                            name_span: token.span,
                            fields,
                        },
                        token.span.join(end),
                    ));
                }
                let mut path = vec![first];
                let mut name_span = token.span;
                while self.eat(&TokenKind::Dot).is_some() {
                    let (name, span) = self.expect_any_ident("expected a name after `.`")?;
                    path.push(name);
                    name_span = span;
                }
                if self.at(&TokenKind::LParen) || self.begins_generic_call() {
                    let (type_arguments, type_argument_span) = self.call_type_arguments()?;
                    self.expect(TokenKind::LParen, "expected `(` after generic arguments")?;
                    let (args, end) = self.expression_list(
                        TokenKind::RParen,
                        "expected `)` after arguments",
                        true,
                    );
                    Ok(self.new_expr(
                        ExprKind::Call {
                            callee: path,
                            name_span,
                            receiver: None,
                            type_arguments,
                            type_argument_span,
                            args,
                        },
                        token.span.join(end),
                    ))
                } else {
                    let end = self.previous().span;
                    Ok(self.new_expr(ExprKind::Path(path), token.span.join(end)))
                }
            }
            TokenKind::Int(text) => {
                let (value, suffix) =
                    parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
                self.bump();
                Ok(self.new_expr(ExprKind::Int { value, suffix }, token.span))
            }
            TokenKind::Float(text) => {
                let normalized = text.replace('_', "");
                let value: f64 = normalized
                    .parse()
                    .map_err(|_| Diagnostic::new("invalid floating-point literal", token.span))?;
                if !value.is_finite() {
                    return Err(Diagnostic::new(
                        "floating-point literal overflows the finite `f64` range",
                        token.span,
                    ));
                }
                let significand = normalized
                    .split_once(['e', 'E'])
                    .map_or(normalized.as_str(), |(significand, _)| significand);
                if value == 0.0
                    && significand
                        .bytes()
                        .any(|digit| matches!(digit, b'1'..=b'9'))
                {
                    return Err(Diagnostic::new(
                        "floating-point literal underflows `f64` to zero",
                        token.span,
                    ));
                }
                self.bump();
                Ok(self.new_expr(
                    ExprKind::Float(crate::ast::FloatLiteral { normalized, value }),
                    token.span,
                ))
            }
            TokenKind::Char(value) => {
                self.bump();
                Ok(self.new_expr(ExprKind::Char(value), token.span))
            }
            TokenKind::String(value) => {
                self.bump();
                Ok(self.new_expr(ExprKind::String(value), token.span))
            }
            _ => Err(Diagnostic::new("expected an expression", token.span)),
        }
    }

    fn optional_control_flow_value(&mut self) -> Result<Option<Expr>, Diagnostic> {
        if self.at(&TokenKind::Semicolon)
            || self.at(&TokenKind::Comma)
            || self.at(&TokenKind::RParen)
            || self.at(&TokenKind::RBracket)
            || self.at(&TokenKind::RBrace)
            || self.at(&TokenKind::Eof)
            || self.line_break_before_current()
        {
            Ok(None)
        } else {
            self.expression(0).map(Some)
        }
    }

    pub(super) fn record_literal_field(&mut self) -> Result<(String, Expr), Diagnostic> {
        let (name, _) = self.expect_any_ident("expected a record field name")?;
        self.expect(TokenKind::Colon, "expected `:` after the field name")?;
        let value = self.expression(0)?;
        Ok((name, value))
    }

    /// Recognizes a complete `name<T>(...)` call before committing `<` to the
    /// generic-call grammar. The closing `>` followed by `(` disambiguates it
    /// from an ordinary comparison without making whitespace significant.
    fn begins_generic_call(&self) -> bool {
        if !self.at(&TokenKind::Lt) {
            return false;
        }
        let mut offset = 1usize;
        let mut brackets = 0usize;
        let mut angles = 1usize;
        loop {
            match self.peek(offset).kind {
                TokenKind::LBracket => brackets += 1,
                TokenKind::RBracket if brackets > 0 => brackets -= 1,
                TokenKind::Lt if brackets == 0 => angles += 1,
                TokenKind::Gt if brackets == 0 => {
                    angles -= 1;
                    if angles == 0 {
                        return self.peek(offset + 1).kind == TokenKind::LParen;
                    }
                }
                TokenKind::Shr if brackets == 0 => {
                    if angles < 2 {
                        return false;
                    }
                    angles -= 2;
                    if angles == 0 {
                        return self.peek(offset + 1).kind == TokenKind::LParen;
                    }
                }
                // These contain an assignment after their leading generic
                // closers, so they cannot directly precede a call's `(`.
                TokenKind::Ge | TokenKind::ShrAssign if brackets == 0 => return false,
                TokenKind::Semicolon if brackets == 0 => return false,
                TokenKind::Eof | TokenKind::LBrace | TokenKind::RBrace => {
                    return false;
                }
                _ => {}
            }
            offset += 1;
        }
    }

    fn call_type_arguments(&mut self) -> Result<(Vec<TypeRef>, Option<Span>), Diagnostic> {
        if !self.begins_generic_call() {
            return Ok((Vec::new(), None));
        }
        let opening = self.bump().span;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.parse_type("expected a type argument after `<`")?.0);
            if self.eat(&TokenKind::Comma).is_some() {
                if let Some(closing) = self.eat_generic_close() {
                    return Ok((arguments, Some(opening.join(closing))));
                }
                continue;
            }
            let closing = self.expect_generic_close("expected `>` after type arguments")?;
            return Ok((arguments, Some(opening.join(closing))));
        }
    }

    pub(super) fn interpolated_expression(&mut self, expression_start: usize) -> Option<Expr> {
        match self.expression(0) {
            Ok(value) => {
                if self.eat(&TokenKind::TemplateExprEnd).is_none() {
                    let error = self.error("expected `}` after the interpolated expression");
                    self.record_recovery_diagnostic(error);
                    let skipped_start = self.current().span.start;
                    self.synchronize_interpolation();
                    self.record_error_region(skipped_start, self.current().span.start);
                    self.eat(&TokenKind::TemplateExprEnd);
                }
                Some(value)
            }
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                self.synchronize_interpolation();
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::TemplateExprEnd);
                None
            }
        }
    }

    pub(super) fn synchronize_interpolation(&mut self) {
        let mut nested_interpolations = 0u32;
        loop {
            match self.current().kind {
                TokenKind::Eof => return,
                TokenKind::TemplateExprEnd if nested_interpolations == 0 => return,
                TokenKind::TemplateExprStart => {
                    nested_interpolations += 1;
                    self.bump();
                }
                TokenKind::TemplateExprEnd => {
                    nested_interpolations -= 1;
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    pub(super) fn recover_required_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
    ) -> Result<Expr, Diagnostic> {
        match parsed {
            Ok(expression) => Ok(expression),
            Err(error) if self.is_expression_recovery_boundary() => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                Ok(self.new_expr(ExprKind::Error, span))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn root_expression(&mut self) -> Expr {
        let expression_start = self.cursor.position();
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(0)
        };
        self.recover_root_expression(parsed, expression_start)
    }

    pub(super) fn recover_root_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
    ) -> Expr {
        match parsed {
            Ok(expression) => expression,
            Err(error) => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                self.synchronize_root_expression(expression_start);
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                self.new_expr(ExprKind::Error, span)
            }
        }
    }

    pub(super) fn synchronize_root_expression(&mut self, expression_start: usize) {
        let target_depth = self.delimiter_depth_before(expression_start);
        let mut depth = self.delimiter_depth_before(self.cursor.position());
        loop {
            let at_same_brace_depth = depth.braces == target_depth.braces;
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::Semicolon) && at_same_brace_depth)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || (self.at(&TokenKind::RBrace) && depth.braces <= target_depth.braces)
                || (self.at(&TokenKind::RParen)
                    && at_same_brace_depth
                    && depth.parentheses <= target_depth.parentheses)
                || (self.at(&TokenKind::RBracket)
                    && at_same_brace_depth
                    && depth.brackets <= target_depth.brackets)
                || (at_same_brace_depth
                    && self.line_break_before_current()
                    && (self.cursor.position() > expression_start || self.is_statement_start()))
                || (self.cursor.position() > expression_start
                    && depth == target_depth
                    && self.is_top_level_start())
            {
                return;
            }
            let kind = self.bump().kind.clone();
            depth.update(&kind);
        }
    }

    pub(super) fn required_expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let expression_start = self.cursor.position();
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(min_precedence)
        };
        self.recover_required_expression(parsed, expression_start)
    }

    pub(super) fn expression_is_missing_before_statement(&self) -> bool {
        if !self.line_break_before_current() {
            return false;
        }
        match &self.current().kind {
            TokenKind::Ident(name) => {
                matches!(
                    name.as_str(),
                    "debug" | "let" | "const" | "var" | "while" | "for"
                ) || assignment_operator(&self.peek(1).kind).is_some()
            }
            _ => false,
        }
    }

    pub(super) fn is_expression_recovery_boundary(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Eof
                | TokenKind::LBrace
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::TemplateExprEnd
        ) || self.at_ident("else")
            || (self.line_break_before_current() && self.is_statement_start())
    }

    pub(super) fn synchronize_delimited_expression(
        &mut self,
        closing: &TokenKind,
        target_depth: DelimiterDepth,
    ) {
        let mut depth = self.delimiter_depth_before(self.cursor.position());
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.at(&TokenKind::TemplateExprEnd)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || self.at_ident("else")
                || self.is_expression_list_boundary(closing, depth, target_depth)
                || (depth == target_depth
                    && self.line_break_before_current()
                    && self.is_statement_start())
            {
                return;
            }
            let kind = self.bump().kind.clone();
            depth.update(&kind);
        }
    }

    pub(super) fn match_arm(&mut self) -> Result<MatchArm, Diagnostic> {
        let token = self.current().clone();
        let pattern_start = token.span;
        let pattern = match token.kind {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                MatchPattern::Wildcard
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                MatchPattern::Bool(true)
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                MatchPattern::Bool(false)
            }
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name) if name == "v" => {
                self.bump();
                let version_span = self.current().span;
                let value = self.expect_string("expected a quoted version after `v`")?;
                MatchPattern::FileVersion(
                    parse_file_version(&value)
                        .map_err(|message| Diagnostic::new(message, version_span))?,
                )
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_foreign_spelling_diagnostic(
                    token.span,
                    "null",
                    crate::migration::ForeignSpellingContext::OptionalValue,
                );
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name)
                if matches!(name.as_str(), "Some" | "Ok" | "Err")
                    && matches!(self.peek(1).kind, TokenKind::LParen) =>
            {
                self.bump();
                self.bump();
                let (binding_name, binding_span) =
                    self.expect_any_ident("expected a binding or `_` in the wrapper pattern")?;
                self.expect(TokenKind::RParen, "expected `)` after the wrapper binding")?;
                let binding = (binding_name != "_").then(|| PatternBinding {
                    id: self.new_value_id(),
                    name: binding_name,
                    name_span: binding_span,
                });
                match name.as_str() {
                    "Some" => MatchPattern::OptionSome(binding),
                    "Ok" => MatchPattern::ResultSuccess(binding),
                    "Err" => MatchPattern::ResultError(binding),
                    _ => unreachable!(),
                }
            }
            TokenKind::Ident(enum_name) => {
                self.bump();
                if self.eat(&TokenKind::Dot).is_some() {
                    let (variant, _) = self.expect_any_ident("expected a variant name")?;
                    let binding = if self.eat(&TokenKind::LParen).is_some() {
                        let (name, name_span) =
                            self.expect_any_ident("expected a payload binding")?;
                        self.expect(TokenKind::RParen, "expected `)` after the payload binding")?;
                        Some(PatternBinding {
                            id: self.new_value_id(),
                            name,
                            name_span,
                        })
                    } else {
                        None
                    };
                    MatchPattern::Enum {
                        enumeration: EnumReference {
                            name: enum_name,
                            span: pattern_start,
                        },
                        variant,
                        binding,
                    }
                } else {
                    return Err(Diagnostic::new(
                        format!(
                            "bare binding `{enum_name}` would match every value; use `Some({enum_name})` or `Ok({enum_name})` to match a wrapper payload"
                        ),
                        pattern_start,
                    ));
                }
            }
            TokenKind::Int(text) => {
                self.bump();
                let (value, suffix) = parse_integer(&text)
                    .map_err(|message| Diagnostic::new(message, pattern_start))?;
                MatchPattern::Int { value, suffix }
            }
            TokenKind::Char(value) => {
                self.bump();
                MatchPattern::Char(value)
            }
            TokenKind::String(value) => {
                self.bump();
                MatchPattern::String(value)
            }
            _ => {
                return Err(Diagnostic::new(
                    "expected an enum variant, string, character, integer, file-version, boolean, `None`, `Some(value)`, `Ok(value)`, `Err(error)`, or `_` pattern",
                    pattern_start,
                ));
            }
        };
        let guard = if self.eat_ident("if").is_some() {
            Some(self.expression(0)?)
        } else {
            None
        };
        self.expect(TokenKind::FatArrow, "expected `=>` after the pattern")?;
        let value = self.expression(0)?;
        let span = pattern_start.join(value.span);
        let arm = MatchArm {
            pattern_id: self.new_pattern_id(),
            pattern,
            guard,
            value,
            span,
        };
        if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
            return Err(self.error("expected `,` between match arms"));
        }
        Ok(arm)
    }

    pub(super) fn expression_list(
        &mut self,
        closing: TokenKind,
        missing_closing_message: &'static str,
        allow_trailing_comma: bool,
    ) -> (Vec<Expr>, Span) {
        let target_depth = self.delimiter_depth_before(self.cursor.position());
        let mut expressions = Vec::new();
        loop {
            let depth = self.delimiter_depth_before(self.cursor.position());
            if self.at(&closing) && depth == target_depth {
                return (expressions, self.bump().span);
            }
            if self.is_expression_list_boundary(&closing, depth, target_depth) {
                self.record_missing(Diagnostic::new(
                    missing_closing_message,
                    self.current().span,
                ));
                return (expressions, self.previous().span);
            }

            let item_start = self.cursor.position();
            let parsed = self.expression(0);
            if let Some(expression) =
                self.recover_expression_list_item(parsed, item_start, target_depth, &closing)
            {
                expressions.push(expression);
                if self.eat(&TokenKind::Comma).is_some() {
                    if !allow_trailing_comma
                        && self.at(&closing)
                        && self.delimiter_depth_before(self.cursor.position()) == target_depth
                    {
                        self.record_missing(Diagnostic::new(
                            "expected an expression after `,`",
                            self.current().span,
                        ));
                    }
                    continue;
                }
                if self.at(&closing)
                    && self.delimiter_depth_before(self.cursor.position()) == target_depth
                {
                    continue;
                }
                self.record_missing(Diagnostic::new(
                    "expected `,` between expressions",
                    self.current().span,
                ));
                if self.is_expression_start() {
                    continue;
                }
                self.synchronize_expression_list_item(target_depth, &closing);
            }
        }
    }

    pub(super) fn recover_expression_list_item(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        item_start: usize,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) -> Option<Expr> {
        match parsed {
            Ok(expression) => Some(expression),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[item_start].span.start;
                self.synchronize_expression_list_item(target_depth, closing);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    pub(super) fn synchronize_expression_list_item(
        &mut self,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) {
        let mut depth = self.delimiter_depth_before(self.cursor.position());
        loop {
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.is_expression_list_boundary(closing, depth, target_depth)
            {
                return;
            }
            let kind = self.bump().kind.clone();
            if kind == TokenKind::Comma && depth == target_depth {
                return;
            }
            depth.update(&kind);
        }
    }

    pub(super) fn is_expression_list_boundary(
        &self,
        closing: &TokenKind,
        depth: DelimiterDepth,
        target: DelimiterDepth,
    ) -> bool {
        if self.at(&TokenKind::Eof) || (self.at(&TokenKind::Semicolon) && depth == target) {
            return true;
        }
        match self.current().kind {
            TokenKind::RParen => {
                *closing != TokenKind::RParen && depth.parentheses <= target.parentheses
            }
            TokenKind::RBracket => {
                *closing != TokenKind::RBracket && depth.brackets <= target.brackets
            }
            TokenKind::RBrace => depth.braces <= target.braces,
            _ => false,
        }
    }

    pub(super) fn delimiter_depth_before(&self, position: usize) -> DelimiterDepth {
        let mut depth = DelimiterDepth {
            parentheses: 0,
            brackets: 0,
            braces: 0,
        };
        for token in &self.cursor.tokens()[..position] {
            depth.update(&token.kind);
        }
        depth
    }

    pub(super) fn if_expression(&mut self, start: Span) -> Result<Expr, Diagnostic> {
        let condition = self.required_expression(0)?;
        let then_expr = self.value_block_expression("expected `{` after the `if` condition")?;
        let else_expr = if self.eat_ident("else").is_none() {
            let error = Diagnostic::new(
                "an `if` expression needs an `else` branch",
                self.current().span,
            );
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            self.new_expr(ExprKind::Error, span)
        } else if self.eat_ident("if").is_some() {
            let nested_start = self.previous().span;
            self.if_expression(nested_start)?
        } else {
            self.value_block_expression("expected `{` after `else`")?
        };
        let span = start.join(else_expr.span);
        Ok(self.new_expr(
            ExprKind::If {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            span,
        ))
    }

    pub(super) fn value_block_expression(
        &mut self,
        message: &'static str,
    ) -> Result<Expr, Diagnostic> {
        if !self.at(&TokenKind::LBrace) {
            let error = self.error(message);
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            return Ok(self.new_expr(ExprKind::Error, span));
        }
        let block = self.block()?;
        let span = block.span;
        if block.trailing_semicolon.is_none()
            && let [crate::ast::Stmt::Expression(expression)] = block.statements.as_slice()
        {
            let mut expression = expression.clone();
            expression.span = span;
            Ok(expression)
        } else {
            Ok(self.value_block(block))
        }
    }

    fn value_block(&mut self, block: crate::ast::Block) -> Expr {
        if let Some(semicolon) = block.trailing_semicolon
            && matches!(
                block.statements.last(),
                Some(crate::ast::Stmt::Expression(_))
            )
        {
            self.diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::ValueBlockSemicolon,
                    "a trailing semicolon does not discard a value block's final expression",
                    semicolon,
                )
                .with_primary_label("this expression still supplies the block's value")
                .with_note(
                    "value blocks use their final expression even when it has a semicolon; ordinary function bodies still require `return`",
                )
                .with_machine_applicable_fix("remove the trailing semicolon", semicolon, ""),
            );
        }
        let span = block.span;
        self.new_expr(ExprKind::Block(block), span)
    }

    pub(super) fn binary_operator(&self) -> Option<(u8, BinaryOp)> {
        Some(match self.current().kind {
            TokenKind::OrOr => (1, BinaryOp::Or),
            TokenKind::AndAnd => (2, BinaryOp::And),
            TokenKind::EqEq => (3, BinaryOp::Eq),
            TokenKind::BangEq => (3, BinaryOp::Ne),
            TokenKind::Lt => (3, BinaryOp::Lt),
            TokenKind::Le => (3, BinaryOp::Le),
            TokenKind::Gt => (3, BinaryOp::Gt),
            TokenKind::Ge => (3, BinaryOp::Ge),
            TokenKind::Or => (4, BinaryOp::BitOr),
            TokenKind::Caret => (5, BinaryOp::BitXor),
            TokenKind::And => (6, BinaryOp::BitAnd),
            TokenKind::Shl => (7, BinaryOp::Shl),
            TokenKind::Shr => (7, BinaryOp::Shr),
            TokenKind::Plus => (8, BinaryOp::Add),
            TokenKind::Minus => (8, BinaryOp::Sub),
            TokenKind::Star => (9, BinaryOp::Mul),
            TokenKind::Slash => (9, BinaryOp::Div),
            TokenKind::Percent => (9, BinaryOp::Rem),
            _ => return None,
        })
    }
}

fn parse_file_version(value: &str) -> Result<[u16; 4], &'static str> {
    let mut components = value.split('.');
    let mut parsed = [0; 4];
    for component in &mut parsed {
        let Some(text) = components.next() else {
            return Err("file-version literals require exactly four decimal components");
        };
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("file-version components must be decimal integers");
        }
        *component = text
            .parse()
            .map_err(|_| "file-version components must fit in `u16`")?;
    }
    if components.next().is_some() {
        return Err("file-version literals require exactly four decimal components");
    }
    Ok(parsed)
}

fn flatten_postfix_receiver(receiver: Expr, members: &mut Vec<String>) -> Expr {
    let Expr { id, kind, span } = receiver;
    match kind {
        ExprKind::Member { receiver, name, .. } => {
            let receiver = flatten_postfix_receiver(*receiver, members);
            members.push(name);
            receiver
        }
        kind => Expr { id, kind, span },
    }
}
