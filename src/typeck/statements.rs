//! Statement, lexical-scope, and callable-control-flow checking.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, Block, Expr, Span, Stmt, SuspensionMode, VariableDecl},
    inference::{Requirements, Type},
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
                        | Stmt::For { .. }
                        | Stmt::Expression(_)
                        | Stmt::Suspend { .. }
                ) {
                    self.error(
                        "`debug` currently supports bindings, expression statements, assignments, `if`, `while`, `for`, and `await`/`retry` statements",
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
                            let resolved = binding.id.and_then(|target| {
                                self.resolve_assignment_operator(
                                    *id, *op, binding.ty, target, *span,
                                )
                            });
                            if let Some(result) = resolved {
                                self.unify(result, binding.ty, *span);
                            } else {
                                self.require_binary_operand(*op, binding.ty, *span);
                            }
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
            Stmt::For {
                binding,
                iterable_value,
                index_value,
                iterable,
                body,
                ..
            } => {
                // Empty literals have no elements from which to infer `T`, but
                // the loop body can still constrain its binding. Seed only
                // that otherwise-ambiguous shape; non-empty and named arrays
                // retain their exact source type (including `[T; N]`).
                let empty_array_hint = matches!(&iterable.kind, crate::ast::ExprKind::Array(values) if values.is_empty())
                    .then(|| {
                        let element = self.fresh_inference(Requirements::none(), None);
                        (Type::Array(self.array_type_id(element)), element)
                    });
                let iterable_ty = self.expr(iterable, empty_array_hint.map(|(array, _)| array));
                let (iterable_ty, element_ty) = match iterable_ty {
                    None => empty_array_hint.unwrap_or_else(|| {
                        let element = self.fresh_inference(Requirements::none(), None);
                        (Type::Array(self.array_type_id(element)), element)
                    }),
                    Some(ty) => match self.shallow_type(ty) {
                        Type::Array(array) => (ty, self.inference.array_element(array)),
                        Type::Set(set) => (ty, self.inference.set_element(set)),
                        Type::Known(id) => match self.inference.type_store().kind(id) {
                            crate::types::TypeKind::Array { element, .. } => {
                                (ty, Type::Known(*element))
                            }
                            crate::types::TypeKind::Set { element, .. } => {
                                (ty, Type::Known(*element))
                            }
                            _ => {
                                let actual = self.type_name(ty);
                                self.error(
                                    format!(
                                        "`for ... in` expects an array or set, but this expression has type `{actual}`"
                                    ),
                                    iterable.span,
                                );
                                let element = self.fresh_inference(Requirements::none(), None);
                                (Type::Array(self.array_type_id(element)), element)
                            }
                        },
                        Type::Variable(variable) => {
                            let element = self.fresh_inference(Requirements::none(), None);
                            let array = Type::Array(self.array_type_id(element));
                            if self.inference.variable_requirements(variable).is_empty() {
                                self.unify(ty, array, iterable.span);
                            } else {
                                let actual = self.type_name(ty);
                                self.error(
                                    format!(
                                        "`for ... in` expects an array or set, but this expression has type `{actual}`"
                                    ),
                                    iterable.span,
                                );
                            }
                            (array, element)
                        }
                        _ => {
                            let actual = self.type_name(ty);
                            self.error(
                                format!(
                                    "`for ... in` expects an array or set, but this expression has type `{actual}`"
                                ),
                                iterable.span,
                            );
                            let element = self.fresh_inference(Requirements::none(), None);
                            (Type::Array(self.array_type_id(element)), element)
                        }
                    },
                };
                self.semantics
                    .resolve_value_type(*iterable_value, iterable_ty);
                self.semantics.resolve_value_type(
                    *index_value,
                    self.core_type(crate::stdlib::CoreTypeId::U32),
                );
                self.semantics.resolve_value_type(binding.id, element_ty);

                let duplicate = self
                    .scopes
                    .iter()
                    .rev()
                    .any(|scope| scope.contains_key(&binding.name))
                    || (!self.is_library_function()
                        && self.declarations.globals.contains_key(&binding.name))
                    || self.is_provider_value_name(&binding.name);
                if duplicate {
                    self.error(
                        format!("variable `{}` is already declared", binding.name),
                        binding.span,
                    );
                }
                self.scopes.push(HashMap::new());
                self.scopes.last_mut().unwrap().insert(
                    binding.name.clone(),
                    Binding {
                        id: Some(binding.id),
                        ty: element_ty,
                        mutable: false,
                        debug_only: self.debug_context.is_debug(),
                    },
                );
                self.with_loop(|checker| checker.block(body, false));
                self.scopes.pop();
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
                returns,
                value,
                span,
            } => {
                if !self.callable.can_suspend() {
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
                                .call_is_provisionally_awaitable(value.id, &self.standard_library);
                            if !supported {
                                self.error("this operation is not awaitable", value.span);
                                return None;
                            }
                            match self.shallow_type(result) {
                                Type::Async(future) => self.inference.async_value(future),
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
                    let expected = if *returns {
                        Some(self.return_ty)
                    } else {
                        binding
                            .as_ref()
                            .and_then(|binding| binding.annotation)
                            .map(|ty| self.syntax_type(ty))
                    };
                    expected.map_or(Some(result), |expected| {
                        self.unify(result, expected, value.span)
                    })
                });
                if let Some(binding) = binding {
                    let ty = result.unwrap_or_else(|| self.error_type());
                    let duplicate = self
                        .scopes
                        .iter()
                        .rev()
                        .any(|scope| scope.contains_key(&binding.name))
                        || (!self.is_library_function()
                            && self.declarations.globals.contains_key(&binding.name))
                        || self.is_provider_value_name(&binding.name);
                    if duplicate {
                        self.error(
                            format!("variable `{}` is already declared", binding.name),
                            binding.span,
                        );
                    }
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
            Stmt::Expression(expr) => {
                self.expr(expr, None);
            }
        }
    }

    pub(super) fn check_return(&mut self, value: Option<&Expr>, span: Span) {
        let returns_none = self.return_ty == self.core_type(crate::stdlib::CoreTypeId::None);
        match (returns_none, self.return_ty, value) {
            (true, _, None) => {}
            (true, _, Some(value)) if !self.callable.is_function() => {
                self.expr(value, None);
                self.error("this lifecycle block cannot return a value", span);
            }
            (_, expected, Some(value)) => {
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
            || (!self.is_library_function()
                && self.declarations.globals.contains_key(&variable.name))
            || self.is_provider_value_name(&variable.name)
        {
            self.error(
                format!("variable `{}` is already declared", variable.name),
                variable.span,
            );
        }
        let expected = variable.annotation.map(|ty| self.syntax_type(ty));
        let mut ty = self
            .expr(&variable.value, expected)
            .unwrap_or_else(|| self.error_type());
        let unsupported_standard = self.standard_type_id(ty).is_some_and(|standard| {
            !self
                .standard_library
                .type_decl(standard)
                .value_usage
                .local_variable
        });
        if unsupported_standard {
            let name = self.type_name(ty);
            self.error(
                format!("local variables cannot currently store `{name}`"),
                variable.span,
            );
            ty = self.error_type();
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

    pub(super) fn binding(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .or_else(|| self.declarations.globals.get(name).copied())
    }

    pub(super) fn binding_for_use(&mut self, name: &str, span: Span) -> Option<Binding> {
        let binding = self.binding(name)?;
        if binding.id == self.layout_value
            && matches!(self.callable, CallableContext::Action(ActionKind::OnAttach))
        {
            self.error(
                "`layout` is only available after `onAttach` has returned it",
                span,
            );
        }
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
