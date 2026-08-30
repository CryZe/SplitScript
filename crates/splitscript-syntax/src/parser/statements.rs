//! Blocks, statements, bindings, and statement-local control flow.

//! Statement grammar.

use super::{
    ASL_TIMER_CONTROL_DIAGNOSTIC, Block, Diagnostic, ExprKind, ForBinding, Parser, RecoveryNode,
    RecoveryNodeKind, Span, Stmt, TokenKind, VariableDecl, assignment_operator, statement_span,
};

impl Parser<'_> {
    pub(super) fn block(&mut self) -> Result<Block, Diagnostic> {
        let start = self
            .expect(TokenKind::LBrace, "expected `{` to start a block")?
            .start;
        let block_depth = self.brace_depth_before(self.cursor.position());
        let mut statements = Vec::new();
        let mut trailing_semicolon = None;
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                let error = self.error("unterminated block");
                self.diagnostics.push(error);
                self.recovery_nodes.push(RecoveryNode {
                    kind: RecoveryNodeKind::Missing,
                    span: self.current().span,
                });
                return Ok(Block {
                    statements,
                    span: Span {
                        start,
                        end: self.current().span.end,
                    },
                    trailing_semicolon,
                });
            }
            let statement_start = self.cursor.position();
            match self.statement() {
                Ok(statement) => {
                    // Error expressions can deliberately stop before a token
                    // owned by their caller, such as the opening brace after
                    // a missing `if` condition. At statement level there is
                    // no caller that can consume a stray opening brace. Never
                    // allow a successfully recovered statement to leave the
                    // block parser at the same token: doing so would append
                    // error nodes forever.
                    if self.cursor.position() == statement_start {
                        let skipped_start = self.current().span.start;
                        self.synchronize_statement(statement_start, block_depth);
                        if self.cursor.position() == statement_start
                            && !self.at(&TokenKind::Eof)
                            && !self.at(&TokenKind::RBrace)
                        {
                            self.bump();
                        }
                        self.record_error_region(skipped_start, self.current().span.start);
                    }
                    trailing_semicolon = (self.previous().kind == TokenKind::Semicolon)
                        .then_some(self.previous().span);
                    statements.push(statement);
                }
                Err(error) => {
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
                    let skipped_start = self.cursor.tokens()[statement_start].span.start;
                    self.synchronize_statement(statement_start, block_depth);
                    let skipped_end = self.current().span.start.max(skipped_start);
                    if skipped_end != skipped_start {
                        self.recovery_nodes.push(RecoveryNode {
                            kind: RecoveryNodeKind::Error,
                            span: Span {
                                start: skipped_start,
                                end: skipped_end,
                            },
                        });
                    }
                }
            }
        }
        let end = self.bump().span.end;
        Ok(Block {
            statements,
            span: Span { start, end },
            trailing_semicolon,
        })
    }

    pub(super) fn statement(&mut self) -> Result<Stmt, Diagnostic> {
        if self.eat_ident("debug").is_some() {
            let start = self.previous().span.start;
            if self.at_ident("debug") {
                return Err(self.error("a statement cannot have more than one `debug` modifier"));
            }
            let statement = self.statement()?;
            let end = statement_span(&statement).end;
            return Ok(Stmt::Debug {
                statement: Box::new(statement),
                span: Span { start, end },
            });
        }
        if self.at_ident("let") || self.at_ident("const") || self.at_ident("var") {
            if self.at_ident("const") || self.at_ident("var") {
                self.record_let_keyword_diagnostic();
            }
            let declaration = self.variable_decl(false)?;
            self.terminator()?;
            return Ok(Stmt::Variable(declaration));
        }
        if self.eat_ident("if").is_some() {
            let start = self.previous().span.start;
            return self.if_statement(start);
        }
        if self.eat_ident("while").is_some() {
            let start = self.previous().span.start;
            let condition = self.root_expression_before_block();
            let body = self.block()?;
            let end = body.span.end;
            return Ok(Stmt::While {
                condition,
                body,
                span: Span { start, end },
            });
        }
        if self.eat_ident("for").is_some() {
            let start = self.previous().span.start;
            let (name, name_span) = self.expect_declared_ident("expected a binding after `for`")?;
            let in_span = self.expect_ident("in")?;
            let iterable = self.root_expression_before_block();
            let body = self.block()?;
            let end = body.span.end;
            return Ok(Stmt::For {
                binding: ForBinding {
                    id: self.new_value_id(),
                    name,
                    span: name_span,
                },
                in_span,
                iterable_value: self.new_value_id(),
                index_value: self.new_value_id(),
                version_value: self.new_value_id(),
                iterable,
                body,
                span: Span { start, end },
            });
        }
        if self.at_ident("timer")
            && self.peek(1).kind == TokenKind::Dot
            && matches!(&self.peek(2).kind, TokenKind::Ident(name) if name == "Run")
            && self.peek(3).kind == TokenKind::Dot
            && matches!(&self.peek(4).kind, TokenKind::Ident(name) if name == "Offset")
            && assignment_operator(&self.peek(5).kind).is_some()
        {
            let target = self.current().span.join(self.peek(4).span);
            return Err(self.migration_diagnostic(ASL_TIMER_CONTROL_DIAGNOSTIC, target));
        }
        if self.at_ident("timer")
            && self.peek(1).kind == TokenKind::Dot
            && matches!(
                &self.peek(2).kind,
                TokenKind::Ident(name) if name == "CurrentTimingMethod"
            )
            && assignment_operator(&self.peek(3).kind).is_some()
        {
            let target = self.current().span.join(self.peek(2).span);
            return Err(self.migration_diagnostic(ASL_TIMER_CONTROL_DIAGNOSTIC, target));
        }
        if let TokenKind::Ident(name) = &self.current().kind
            && let Some(op) = assignment_operator(&self.peek(1).kind)
        {
            let name = name.clone();
            let start = self.bump().span.start;
            self.bump();
            let value = self.root_expression();
            self.terminator()?;
            return Ok(Stmt::Assign {
                id: self.new_assignment_id(),
                name,
                op,
                span: Span {
                    start,
                    end: self.previous().span.end,
                },
                value,
            });
        }
        let expr = self.root_expression();
        if let Some(op) = assignment_operator(&self.current().kind) {
            let operator_span = self.bump().span;
            let value = self.root_expression();
            self.terminator()?;
            let span = expr.span.join(self.previous().span);
            if matches!(&expr.kind, ExprKind::Path(path) if path.len() == 2 && path[0] == "current")
            {
                return Ok(Stmt::StateAssign {
                    id: self.new_assignment_id(),
                    target: expr,
                    op,
                    value,
                    span,
                });
            }
            if matches!(&expr.kind, ExprKind::Path(path) if path.first().is_some_and(|root| root == "old"))
            {
                return Err(Diagnostic::new("`old` state is read-only", expr.span)
                    .with_primary_label("history cannot be changed")
                    .with_note("assign to the corresponding `current` field instead")
                    .with_migration_topic("asl.state.mutable-current"));
            }
            if let Some(op) = op {
                if !matches!(expr.kind, ExprKind::Index { .. }) {
                    return Err(Diagnostic::new(
                        "only variables and array elements can be assigned",
                        expr.span,
                    ));
                }
                return Ok(Stmt::IndexAssign {
                    id: self.new_assignment_id(),
                    target: expr,
                    op,
                    value,
                    span,
                });
            }
            let ExprKind::Index {
                receiver, index, ..
            } = expr.kind
            else {
                return Err(Diagnostic::new(
                    "only variables and array elements can be assigned",
                    expr.span,
                ));
            };
            return Ok(Stmt::Expression(self.new_expr(
                ExprKind::Call {
                    callee: vec!["set".to_owned()],
                    // The catalog method is compiler-inserted syntax sugar,
                    // so it has no source identifier to highlight or navigate.
                    name_span: Span {
                        start: operator_span.start,
                        end: operator_span.start,
                    },
                    receiver: Some(receiver),
                    type_arguments: Vec::new(),
                    type_argument_span: None,
                    args: vec![*index, value],
                },
                span,
            )));
        }
        self.terminator()?;
        Ok(Stmt::Expression(expr))
    }

    pub(super) fn if_statement(&mut self, start: usize) -> Result<Stmt, Diagnostic> {
        let condition = self.root_expression_before_block();
        let then_block = self.block()?;
        let else_block = if self.eat_ident("else").is_some() {
            if self.eat_ident("if").is_some() {
                let nested_start = self.previous().span.start;
                let nested = self.if_statement(nested_start)?;
                let span = match &nested {
                    Stmt::If { span, .. } => *span,
                    _ => unreachable!(),
                };
                Some(Block {
                    statements: vec![nested],
                    span,
                    trailing_semicolon: None,
                })
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map_or(then_block.span.end, |block| block.span.end);
        Ok(Stmt::If {
            condition,
            then_block,
            else_block,
            span: Span { start, end },
        })
    }

    pub(super) fn variable_decl(
        &mut self,
        allow_attachment_scoped: bool,
    ) -> Result<VariableDecl, Diagnostic> {
        let keyword = self.bump().clone();
        if self.at_ident("mut") {
            let span = self.current().span;
            self.record_foreign_spelling_diagnostic(
                span,
                "mut",
                crate::migration::ForeignSpellingContext::VariableModifier,
            );
            self.bump();
        }
        let (name, name_span) = self.expect_declared_ident("expected a variable name")?;
        let annotation = if self.eat(&TokenKind::Colon).is_some() {
            Some(self.parse_type_before_assignment("expected a type name")?.0)
        } else {
            None
        };
        let value = if self.eat(&TokenKind::Assign).is_some() {
            Some(self.root_expression())
        } else if allow_attachment_scoped {
            None
        } else {
            return Err(self.error("expected `=` in variable declaration"));
        };
        let end = value
            .as_ref()
            .map_or(self.previous().span.end, |value| value.span.end);
        Ok(VariableDecl {
            id: self.new_value_id(),
            name,
            name_span,
            documentation: None,
            mutable: true,
            debug_only: false,
            annotation,
            span: Span {
                start: keyword.span.start,
                end: end.max(name_span.end),
            },
            value,
        })
    }
}
