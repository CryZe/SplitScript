//! Statement, lexical-scope, and callable-control-flow checking.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, Block, Expr, Span, Stmt, SuspensionMode, VariableDecl},
    inference::Type,
    stdlib::StdlibTypeId,
};

use super::{
    Checker,
    context::{CallableContext, DebugContext, ExpressionMode, NonePolicy},
    declarations::Binding,
};

impl Checker {
    pub(super) fn block(&mut self, block: &Block, nested: bool) {
        if nested {
            self.scopes.push(HashMap::new());
        }
        for statement in &block.statements {
            self.statement(statement);
        }
        if nested {
            self.scopes.pop();
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Debug {
                statement: inner,
                span,
            } => {
                if !matches!(
                    inner.as_ref(),
                    Stmt::Variable(_)
                        | Stmt::Assign { .. }
                        | Stmt::If { .. }
                        | Stmt::While { .. }
                        | Stmt::Expression(_)
                        | Stmt::Suspend { .. }
                ) {
                    self.error(
                        "`debug` currently supports bindings, expression statements, assignments, `if`, `while`, and `await`/`retry` statements",
                        *span,
                    );
                }
                self.with_debug_context(DebugContext::DebugOnly, |checker| {
                    checker.statement(inner);
                });
            }
            Stmt::Variable(variable) => self.variable(variable),
            Stmt::Assign {
                id,
                name,
                op,
                value,
                span,
            } => {
                let binding = self.binding_for_use(name, *span);
                match binding {
                    Some(binding) if !binding.mutable => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                        }
                        self.error(format!("cannot assign to constant `{name}`"), *span)
                    }
                    Some(binding) => {
                        if let Some(target) = binding.id {
                            self.semantics.resolve_assignment(*id, target);
                        }
                        if self.expr(value, Some(binding.ty)).is_some()
                            && let Some(op) = op
                        {
                            self.require_binary_operand(*op, binding.ty, *span);
                        }
                    }
                    None => self.error(format!("unknown variable `{name}`"), *span),
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.expr(
                    condition,
                    Some(self.core_type(crate::stdlib::CoreTypeId::Bool)),
                );
                self.block(then_block, true);
                if let Some(else_block) = else_block {
                    self.block(else_block, true);
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.expr(
                    condition,
                    Some(self.core_type(crate::stdlib::CoreTypeId::Bool)),
                );
                self.with_loop(|checker| checker.block(body, true));
            }
            Stmt::Break { span } => {
                if !self.loops.is_inside() {
                    self.error("`break` is only available inside a loop", *span);
                }
            }
            Stmt::Continue { span } => {
                if !self.loops.is_inside() {
                    self.error("`continue` is only available inside a loop", *span);
                }
            }
            Stmt::Return { value, span } => self.check_return(value.as_ref(), *span),
            Stmt::Throw { error, span } => {
                if self.failure.result().is_none() {
                    self.error(
                        "`throw` needs a function returning `T!` or an explicit catch boundary",
                        *span,
                    );
                }
                self.expr(error, Some(self.standard_type(StdlibTypeId::String)));
            }
            Stmt::Suspend {
                mode,
                binding,
                value,
                span,
            } => {
                if !matches!(self.callable, CallableContext::Action(ActionKind::OnAttach)) {
                    let keyword = match mode {
                        SuspensionMode::Await => "await",
                        SuspensionMode::Retry => "retry",
                    };
                    self.error(
                        format!("`{keyword}` is only available inside `onAttach`"),
                        *span,
                    );
                }
                let result = self
                    .with_expression_mode(ExpressionMode::SuspensionOperand, |checker| {
                        checker.expr(value, None)
                    });
                let result = result.and_then(|result| {
                    let result = match mode {
                        SuspensionMode::Await => {
                            let supported = self
                                .semantics
                                .standard_library_item(value.id)
                                .map(|item| self.standard_library.item(item))
                                .is_some_and(|item| {
                                    item.operation_semantics().suspension.is_awaitable()
                                });
                            if !supported {
                                self.error("this operation is not awaitable", value.span);
                                return None;
                            }
                            match self.shallow_type(result) {
                                Type::Result(result) => self.inference.result_value(result),
                                result => result,
                            }
                        }
                        SuspensionMode::Retry => match self.shallow_type(result) {
                            Type::Result(result) => self.inference.result_value(result),
                            _ => {
                                self.error(
                                    "`retry` expects an expression of type `T!`",
                                    value.span,
                                );
                                return None;
                            }
                        },
                    };
                    let expected = binding
                        .as_ref()
                        .and_then(|binding| binding.annotation)
                        .map(|ty| self.syntax_type(ty));
                    expected.map_or(Some(result), |expected| {
                        self.unify(result, expected, value.span)
                    })
                });
                if let (Some(binding), Some(ty)) = (binding, result) {
                    let duplicate = self
                        .scopes
                        .iter()
                        .rev()
                        .any(|scope| scope.contains_key(&binding.name))
                        || self.declarations.globals.contains_key(&binding.name)
                        || self.is_provider_value_name(&binding.name);
                    if duplicate {
                        self.error(
                            format!("variable `{}` is already declared", binding.name),
                            binding.span,
                        );
                    }
                    if ty == self.core_type(crate::stdlib::CoreTypeId::Void) {
                        self.error("a suspended binding needs a value", binding.span);
                    } else {
                        self.semantics.resolve_value_type(binding.id, ty);
                        self.scopes.last_mut().unwrap().insert(
                            binding.name.clone(),
                            Binding {
                                id: Some(binding.id),
                                ty,
                                mutable: true,
                                debug_only: self.debug_context.is_debug(),
                            },
                        );
                    }
                }
            }
            Stmt::Expression(expr) => {
                self.expr(expr, None);
            }
        }
    }

    pub(super) fn check_return(&mut self, value: Option<&Expr>, span: Span) {
        let returns_void = self.return_ty == self.core_type(crate::stdlib::CoreTypeId::Void);
        match (returns_void, self.return_ty, value) {
            (true, _, None) => {}
            (true, _, Some(value)) => {
                self.expr(value, None);
                self.error(
                    format!("{} cannot return a value", self.callable.description()),
                    span,
                );
            }
            (false, expected, Some(value)) => {
                let policy = if !self.callable.is_function()
                    && matches!(
                        self.callable.action(),
                        Some(ActionKind::IsLoading | ActionKind::GameTime)
                    ) {
                    NonePolicy::DomainNullable
                } else {
                    NonePolicy::OptionalOnly
                };
                self.with_none_policy(policy, |checker| {
                    checker.expr(value, Some(expected));
                });
            }
            (false, _, None)
                if !self.callable.is_function()
                    && matches!(
                        self.callable.action(),
                        Some(
                            ActionKind::Start
                                | ActionKind::Split
                                | ActionKind::Reset
                                | ActionKind::IsLoading
                                | ActionKind::GameTime
                        )
                    ) => {}
            (false, expected, None) => {
                let expected = self.type_name(expected);
                self.error(
                    format!("expected a return value of type `{expected}`"),
                    span,
                );
            }
        }
    }

    fn variable(&mut self, variable: &VariableDecl) {
        let duplicate = self
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(&variable.name));
        if duplicate
            || self.declarations.globals.contains_key(&variable.name)
            || self.is_provider_value_name(&variable.name)
        {
            self.error(
                format!("variable `{}` is already declared", variable.name),
                variable.span,
            );
        }
        let expected = variable.annotation.map(|ty| self.syntax_type(ty));
        if let Some(ty) = self.expr(&variable.value, expected) {
            let unsupported_standard = self.standard_type_id(ty).is_some_and(|standard| {
                !self
                    .standard_library
                    .type_decl(standard)
                    .value_usage
                    .local_variable
            });
            if ty == self.core_type(crate::stdlib::CoreTypeId::Void) || unsupported_standard {
                let ty = self.type_name(ty);
                self.error(
                    format!("local variables cannot currently store `{ty}`"),
                    variable.span,
                );
                return;
            }
            self.semantics.resolve_value_type(variable.id, ty);
            self.scopes.last_mut().unwrap().insert(
                variable.name.clone(),
                Binding {
                    id: Some(variable.id),
                    ty,
                    mutable: variable.mutable,
                    debug_only: self.debug_context.is_debug() || variable.debug_only,
                },
            );
        }
    }

    fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .or_else(|| self.declarations.globals.get(name).copied())
    }

    pub(super) fn binding_for_use(&mut self, name: &str, span: Span) -> Option<Binding> {
        let binding = self.binding(name)?;
        if binding.debug_only && !self.debug_context.is_debug() {
            self.error(
                format!("debug-only binding `{name}` can only be used from debug code"),
                span,
            );
        }
        Some(binding)
    }

    pub(super) fn bind_pattern_value(
        &mut self,
        binding: &crate::ast::PatternBinding,
        ty: Type,
        span: Span,
    ) {
        if self.is_provider_value_name(&binding.name) {
            self.error(
                format!("`{}` is reserved by the state provider", binding.name),
                span,
            );
        }
        self.semantics.resolve_value_type(binding.id, ty);
        self.scopes.last_mut().unwrap().insert(
            binding.name.clone(),
            Binding {
                id: Some(binding.id),
                ty,
                mutable: false,
                debug_only: self.debug_context.is_debug(),
            },
        );
    }
}
