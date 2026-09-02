//! Statement, lexical-scope, and callable-control-flow checking.

use std::collections::HashMap;

use crate::{
    ast::{ActionKind, Block, Expr, ExprKind, Span, Stmt, SuspensionMode, VariableDecl},
    inference::{Requirements, Type},
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

    pub(super) fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Debug {
                statement: inner,
                span,
            } => {
                if contains_control_flow_expression(inner) {
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
                        if let Some(op) = op {
                            if let Some(right_type) = self.expr(value, None)
                                && !self.diagnose_string_compound_assignment(
                                    *op, binding.ty, right_type, *span,
                                )
                            {
                                let resolved = binding.id.and_then(|target| {
                                    self.resolve_assignment_operator(
                                        *id, *op, binding.ty, right_type, target, *span,
                                    )
                                });
                                if let Some(result) = resolved {
                                    if let Some(declaration_span) = binding.declaration_span {
                                        let binding_type = self.type_name(binding.ty);
                                        self.with_expected_type_source(
                                            super::ExpectedTypeSource {
                                                span: declaration_span,
                                                label: format!(
                                                    "variable `{name}` has type `{binding_type}`"
                                                ),
                                            },
                                            |checker| {
                                                checker.unify_expected(result, binding.ty, *span);
                                            },
                                        );
                                    } else {
                                        self.unify_expected(result, binding.ty, *span);
                                    }
                                } else if let Some(operand_type) =
                                    self.unify(binding.ty, right_type, *span)
                                {
                                    self.require_binary_operand(*op, operand_type, *span);
                                }
                            }
                        } else {
                            if let Some(declaration_span) = binding.declaration_span {
                                let binding_type = self.type_name(binding.ty);
                                self.with_expected_type_source(
                                    super::ExpectedTypeSource {
                                        span: declaration_span,
                                        label: format!(
                                            "variable `{name}` has type `{binding_type}`"
                                        ),
                                    },
                                    |checker| checker.expr(value, Some(binding.ty)),
                                );
                            } else {
                                self.expr(value, Some(binding.ty));
                            }
                        }
                    }
                    None => self.error(format!("unknown variable `{name}`"), *span),
                }
            }
            Stmt::StateAssign {
                id,
                target,
                op,
                value,
                span,
            } => {
                let target_type = self.expr(target, None);
                let field = match &target.kind {
                    crate::ast::ExprKind::Path(path) => path.get(1),
                    _ => None,
                }
                .and_then(|name| self.visible_state_field(name));
                let Some((field, field_type)) = field else {
                    self.expr(value, None);
                    return;
                };
                self.semantics.resolve_assignment(*id, field);
                let declaration_span = self.declarations.state_field_spans[&field];
                if let Some(op) = op {
                    let Some(right_type) = self.expr(value, None) else {
                        return;
                    };
                    if self.diagnose_string_compound_assignment(*op, field_type, right_type, *span)
                    {
                        return;
                    }
                    let resolved = self.resolve_state_assignment_operator(
                        *id, *op, field_type, right_type, field, *span,
                    );
                    if let Some(result) = resolved {
                        let field_type_name = self.type_name(field_type);
                        self.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: declaration_span,
                                label: format!("state field has type `{field_type_name}`"),
                            },
                            |checker| checker.unify_expected(result, field_type, *span),
                        );
                    } else if let Some(operand_type) = self.unify(field_type, right_type, *span) {
                        self.require_binary_operand(*op, operand_type, *span);
                    }
                } else {
                    let field_type_name = self.type_name(field_type);
                    self.with_expected_type_source(
                        super::ExpectedTypeSource {
                            span: declaration_span,
                            label: format!("state field has type `{field_type_name}`"),
                        },
                        |checker| {
                            checker.expr(value, target_type.or(Some(field_type)));
                        },
                    );
                }
            }
            Stmt::IndexAssign {
                id,
                target,
                op,
                value,
                span,
            } => {
                let Some(element_type) = self.expr(target, None) else {
                    self.expr(value, None);
                    return;
                };
                let Some(right_type) = self.expr(value, None) else {
                    return;
                };
                if self.diagnose_string_compound_assignment(*op, element_type, right_type, *span) {
                    return;
                }
                if let Some(result) = self.resolve_index_assignment_operator(
                    *id,
                    *op,
                    element_type,
                    right_type,
                    target.id,
                    *span,
                ) {
                    self.unify(result, element_type, *span);
                } else if let Some(operand_type) = self.unify(element_type, right_type, *span) {
                    self.require_binary_operand(*op, operand_type, *span);
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
                let constraints = self.layout_constraints(condition);
                let inverse_constraints = self.inverse_layout_constraints(condition);
                self.with_layout_constraints(constraints.as_deref(), |checker| {
                    checker.block(then_block, true);
                });
                if let Some(else_block) = else_block {
                    self.with_layout_constraints(inverse_constraints.as_deref(), |checker| {
                        checker.block(else_block, true);
                    });
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
                version_value,
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
                let iterable_ty = iterable_ty.unwrap_or_else(|| {
                    empty_array_hint.map_or_else(
                        || {
                            let element = self.fresh_inference(Requirements::none(), None);
                            Type::Array(self.array_type_id(element))
                        },
                        |(array, _)| array,
                    )
                });
                let mut iterable_ty = self.shallow_type(iterable_ty);
                // Source-defined methods intentionally omit catalog-generic
                // annotations at the parser boundary and let ordinary
                // inference recover them. When the iterable is that method's
                // still-unbound receiver, its catalog owner supplies the
                // concrete `Iterable` constructor. This is the same
                // contextual receiver constraint used by deferred member
                // resolution, applied before projecting `Iterable.Item`.
                if matches!(iterable_ty, Type::Variable(_))
                    && let crate::typeck::CallableContext::LibraryFunction(item) = self.callable
                    && let crate::stdlib::StdlibOwner::TypeConstructor(constructor) =
                        self.standard_library.item(item).owner
                    && self.standard_library.type_constructor_has_capability(
                        constructor,
                        crate::stdlib::StdlibCapabilityId::Iterable,
                    )
                {
                    let declaration = self.standard_library.type_constructor(constructor);
                    let variables = declaration
                        .parameters
                        .iter()
                        .map(|parameter| {
                            let requirements = parameter.constraints.iter().fold(
                                Requirements::none(),
                                |requirements, constraint| {
                                    requirements | Requirements::capability(*constraint)
                                },
                            );
                            (parameter.name, self.fresh_inference(requirements, None))
                        })
                        .collect::<std::collections::HashMap<_, _>>();
                    let receiver = self.catalog_application_type(
                        constructor,
                        vec![variables[declaration.parameters[0].name]],
                    );
                    self.unify(iterable_ty, receiver, iterable.span);
                    iterable_ty = self.shallow_type(receiver);
                }
                let iterable_capability = crate::stdlib::StdlibCapabilityId::Iterable;
                let iterator_capability = crate::stdlib::StdlibCapabilityId::Iterator;
                let mut consumes_iterator = false;
                let mut converts_iterable = false;
                let element_ty = if matches!(iterable_ty, Type::Variable(_)) {
                    // An associated cursor such as `T.Iterator` already carries
                    // the `Iterator` constraint declared by `Iterable`. Preserve
                    // that stronger protocol instead of adding an unrelated
                    // `Iterable` requirement merely because its concrete type is
                    // not known until a generic call is specialized.
                    let capability = match iterable_ty {
                        Type::Variable(variable)
                            if self.standard_library.capabilities_satisfy(
                                self.inference.variable_requirements(variable).as_slice(),
                                iterator_capability,
                            ) =>
                        {
                            consumes_iterator = true;
                            iterator_capability
                        }
                        _ => {
                            converts_iterable = true;
                            iterable_capability
                        }
                    };
                    self.inference
                        .require(
                            iterable_ty,
                            Requirements::capability(capability),
                        )
                        .ok()
                        .map(|()| {
                            self.inference.associated_type(
                                iterable_ty,
                                capability,
                                "Item",
                            )
                        })
                } else {
                    self.constructed_field_receiver(iterable_ty).and_then(
                        |(constructor, arguments)| {
                        let declaration = self.standard_library.type_constructor(constructor);
                        if self.standard_library.type_constructor_has_capability(
                            constructor, iterator_capability,
                        ) {
                            consumes_iterator = true;
                        } else if self.standard_library.type_constructor_has_capability(
                            constructor, iterable_capability,
                        ) {
                        } else {
                            return None;
                        }
                        let variables = declaration
                            .parameters
                            .iter()
                            .zip(arguments)
                            .map(|(parameter, argument)| (parameter.name, argument))
                            .collect();
                        declaration
                            .associated_types
                            .iter()
                            .find(|associated| associated.name == "Item")
                            .map(|associated| self.catalog_type(associated.value, &variables))
                    },
                    )
                }
                .unwrap_or_else(|| {
                        let actual = self.type_name(iterable_ty);
                        self.error(
                            format!(
                                "`for ... in` requires an `Iterable` or `Iterator` value, but this expression has type `{actual}`"
                            ),
                            iterable.span,
                        );
                        self.fresh_inference(Requirements::none(), None)
                    });
                // A literal range keeps its upper bound directly in the
                // compiler-owned iterable slot. This lets the backend lower a
                // direct range loop without allocating the first-class range
                // object that is needed when a range escapes into a value.
                let iterable_storage_ty = if converts_iterable {
                    self.inference
                        .associated_type(iterable_ty, iterable_capability, "Iterator")
                } else if !consumes_iterator
                    && matches!(iterable.kind, crate::ast::ExprKind::Range { .. })
                {
                    element_ty
                } else {
                    iterable_ty
                };
                self.semantics
                    .resolve_value_type(*iterable_value, iterable_storage_ty);
                let is_range = match self.shallow_type(iterable_ty) {
                    Type::Range(_) => true,
                    Type::Known(id) => matches!(
                        self.inference.type_store().kind(id),
                        crate::types::TypeKind::Range { .. }
                    ),
                    _ => false,
                };
                let index_ty = if consumes_iterator || converts_iterable {
                    self.catalog_application_type(
                        crate::stdlib::StdlibTypeConstructorId::IteratorStep,
                        vec![element_ty],
                    )
                } else if is_range {
                    element_ty
                } else {
                    self.core_type(crate::stdlib::CoreTypeId::U32)
                };
                self.semantics.resolve_value_type(*index_value, index_ty);
                self.semantics.resolve_value_type(
                    *version_value,
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
                        declaration_span: Some(binding.span),
                    },
                );
                self.with_loop(|checker| checker.block(body, false));
                self.scopes.pop();
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
                        format!(
                            "`{keyword}` is only available inside `onAttach`, `whileAttached`, or an inferred async function"
                        ),
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
                        let source = if *returns {
                            self.return_type_source.clone()
                        } else {
                            binding.as_ref().and_then(|binding| {
                                binding.annotation.map(|_| {
                                    let expected_name = self.type_name(expected);
                                    super::ExpectedTypeSource {
                                        span: binding.span,
                                        label: format!(
                                            "variable `{}` is declared as `{expected_name}`",
                                            binding.name
                                        ),
                                    }
                                })
                            })
                        };
                        if let Some(source) = source {
                            self.with_expected_type_source(source, |checker| {
                                checker.unify_expected(result, expected, value.span)
                            })
                        } else {
                            self.unify_expected(result, expected, value.span)
                        }
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
                            declaration_span: Some(binding.span),
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
                    checker.with_expression_mode(ExpressionMode::DirectReturn, |checker| {
                        if let Some(source) = checker.return_type_source.clone() {
                            checker.with_expected_type_source(source, |checker| {
                                checker.expr(value, Some(expected));
                            });
                        } else {
                            checker.expr(value, Some(expected));
                        }
                    });
                });
            }
            (false, _, None)
                if !self.callable.is_function()
                    && matches!(
                        self.callable.action(),
                        Some(
                            ActionKind::Start
                                | ActionKind::SelectProcess
                                | ActionKind::WhileAttached
                                | ActionKind::Split
                                | ActionKind::Reset
                                | ActionKind::IsLoading
                                | ActionKind::GameTime
                        )
                    ) => {}
            (false, expected, None) => {
                let expected = self.type_name(expected);
                let mut diagnostic = crate::Diagnostic::type_error(
                    format!("expected a return value of type `{expected}`"),
                    span,
                )
                .with_primary_label("this return has no value");
                if let Some(source) = self.return_type_source.clone() {
                    diagnostic = diagnostic.with_secondary_label(source.span, source.label);
                }
                self.errors.push(diagnostic);
            }
        }
    }

    fn variable(&mut self, variable: &VariableDecl) {
        let value = variable
            .value
            .as_ref()
            .expect("local variables have initializers");
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
        let inferred = if let Some(expected) = expected {
            let expected_name = self.type_name(expected);
            self.with_expected_type_source(
                super::ExpectedTypeSource {
                    span: variable.name_span,
                    label: format!(
                        "variable `{}` is declared as `{expected_name}`",
                        variable.name
                    ),
                },
                |checker| checker.expr(value, Some(expected)),
            )
        } else {
            self.expr(value, None)
        };
        let mut ty = inferred.unwrap_or_else(|| self.error_type());
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
                declaration_span: Some(variable.name_span),
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
        if name == "layout"
            && binding.id == self.layout_value
            && matches!(self.callable, CallableContext::Action(ActionKind::OnAttach))
            && !self.layout_available_in_on_attach
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
        if self.scopes.last().unwrap().contains_key(&binding.name) {
            self.error(
                format!("pattern binds `{}` more than once", binding.name),
                binding.name_span,
            );
            return;
        }
        self.semantics.resolve_value_type(binding.id, ty);
        self.scopes.last_mut().unwrap().insert(
            binding.name.clone(),
            Binding {
                id: Some(binding.id),
                ty,
                mutable: false,
                debug_only: self.debug_context.is_debug(),
                declaration_span: Some(binding.name_span),
            },
        );
    }
}

fn contains_control_flow_expression(statement: &Stmt) -> bool {
    #[derive(Default)]
    struct Finder(bool);

    impl<'ast> crate::visit::Visitor<'ast> for Finder {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if matches!(
                expression.kind,
                ExprKind::Break(_) | ExprKind::Continue | ExprKind::Return(_) | ExprKind::Throw(_)
            ) {
                self.0 = true;
            } else {
                crate::visit::walk_expr(self, expression);
            }
        }
    }

    let mut finder = Finder::default();
    crate::visit::Visitor::visit_stmt(&mut finder, statement);
    finder.0
}
