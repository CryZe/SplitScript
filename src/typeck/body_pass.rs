//! Global initializer, state-expression, function, and action body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{ActionKind, FunctionId, Program, StateSource},
    inference::Type,
    stdlib::{CoreTypeId, StdlibTypeId},
};

use super::{
    Checker,
    context::{CallableContext, DebugContext, ExpressionMode, FailureContext},
    control_flow::{contains_propagation, definitely_returns, is_constant},
    declarations::Binding,
};

pub(super) fn check(checker: &mut Checker, program: &Program) {
    check_global_initializers(checker, program);
    check_function_bodies(checker, program);
    check_state_expressions(checker, program);
    check_action_bodies(checker, program);
}

fn check_global_initializers(checker: &mut Checker, program: &Program) {
    for global in &program.globals {
        if checker.is_provider_value_name(&global.name) {
            checker.error(
                format!("`{}` is reserved by the state provider", global.name),
                global.span,
            );
            continue;
        }
        if checker.declarations.globals.contains_key(&global.name) {
            checker.error(
                format!("duplicate global variable `{}`", global.name),
                global.span,
            );
            continue;
        }
        if !is_constant(&global.value, &checker.resolutions) {
            checker.error(
                "global initializers must be literal values composed from None, numbers, booleans, strings, payload-free enums, records, or arrays",
                global.value.span,
            );
        }
        let inferred = checker.with_debug_context(
            DebugContext::from_declaration(global.debug_only),
            |checker| {
                let expected = global.annotation.map(|ty| checker.syntax_type(ty));
                checker.expr(&global.value, expected)
            },
        );
        if let Some(ty) = inferred {
            let unsupported_standard = checker.standard_type_id(ty).is_some_and(|standard| {
                standard != StdlibTypeId::String
                    && !checker
                        .standard_library
                        .type_decl(standard)
                        .value_usage
                        .global_variable
            });
            if unsupported_standard
                || matches!(ty, Type::Result(_))
                || matches!(ty, Type::Option(_))
                    && !matches!(global.value.kind, crate::ast::ExprKind::None)
            {
                let ty = checker.type_name(ty);
                checker.error(
                    format!("global variables cannot currently store `{ty}`"),
                    global.span,
                );
            }
            checker.semantics.resolve_value_type(global.id, ty);
            checker.declarations.globals.insert(
                global.name.clone(),
                Binding {
                    id: Some(global.id),
                    ty,
                    mutable: global.mutable,
                    debug_only: global.debug_only,
                },
            );
        }
    }
}

fn check_state_expressions(checker: &mut Checker, program: &Program) {
    checker.with_expression_mode(ExpressionMode::StateSource, |checker| {
        for field in program.state.as_ref().unwrap().all_fields() {
            let StateSource::Expression(expression) = &field.source else {
                continue;
            };
            let field_type = checker.declarations.state_fields[&field.name].1;
            let boundary = contains_propagation(expression)
                .then(|| Type::Result(checker.inference.result_type(field_type)));
            let (actual, failure) = checker.with_failure_context(
                boundary.map_or(FailureContext::None, FailureContext::boundary),
                |checker| checker.expr(expression, None),
            );
            let used_propagation = failure.propagated();
            let Some(actual) = actual else {
                continue;
            };
            let actual = checker.shallow_type(actual);
            let poll_result = if used_propagation {
                let boundary = boundary.expect("propagation syntax creates a failure boundary");
                if matches!(actual, Type::Result(_)) {
                    checker.error(
                        "a state expression using `?` must produce the field value, not another result",
                        expression.span,
                    );
                } else {
                    checker.unify(actual, field_type, expression.span);
                    checker.expect_expression(
                        expression.id,
                        actual,
                        Some(boundary),
                        expression.span,
                    );
                }
                boundary
            } else if let Type::Result(result) = actual {
                let value = checker.inference.result_value(result);
                checker.unify(value, field_type, expression.span);
                actual
            } else {
                checker.unify(actual, field_type, expression.span);
                let result = Type::Result(checker.inference.result_type(actual));
                checker.expect_expression(expression.id, actual, Some(result), expression.span);
                result
            };
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    });
}

fn check_function_bodies(checker: &mut Checker, program: &Program) {
    for component in super::function_graph::dependency_order(program) {
        checker.active_function_component = component.functions.iter().copied().collect();
        for function_id in &component.functions {
            let function = program
                .functions
                .iter()
                .find(|function| function.id == *function_id)
                .expect("function graph identities belong to source declarations");
            check_function_body(checker, function);
        }
        // Member paths participate in signature inference. Resolve them while
        // this component's inference variables are still ordinary unbound
        // roots; once generalized they deliberately stop accepting concrete
        // bindings from later call sites.
        checker.resolve_deferred_member_paths();
        generalize_component(checker, &component.functions);
        checker.active_function_component.clear();
    }
}

fn check_function_body(checker: &mut Checker, function: &crate::ast::FunctionDecl) {
    checker.with_debug_context(
        DebugContext::from_declaration(function.debug_only),
        |checker| {
            let signature = checker.declarations.function_signatures[&function.id].clone();
            let failure = match checker.shallow_type(signature.completion) {
                result @ Type::Result(_) => FailureContext::boundary(result),
                _ => FailureContext::None,
            };
            let callable = checker
                .standard_library
                .items()
                .iter()
                .find_map(|item| match item.implementation {
                    crate::stdlib::Implementation::LibraryBody { function_name, .. }
                        if function_name == function.name =>
                    {
                        Some(CallableContext::LibraryFunction(item.id))
                    }
                    _ => None,
                })
                .unwrap_or(CallableContext::Function);
            checker.with_callable_context(callable, signature.completion, failure, |checker| {
                checker.scopes.clear();
                checker.scopes.push(HashMap::new());
                for (parameter, ty) in function.params.iter().zip(signature.params.iter().copied())
                {
                    if checker.is_provider_value_name(&parameter.name) {
                        checker.error(
                            format!("`{}` is reserved by the state provider", parameter.name),
                            parameter.span,
                        );
                    }
                    let duplicate = checker.scopes[0]
                        .insert(
                            parameter.name.clone(),
                            Binding {
                                id: Some(parameter.id),
                                ty,
                                mutable: true,
                                debug_only: checker.debug_context.is_debug(),
                            },
                        )
                        .is_some();
                    if duplicate {
                        checker.error(
                            format!("duplicate parameter `{}`", parameter.name),
                            parameter.span,
                        );
                    }
                }
                checker.block(&function.body, false);
                if signature.completion != checker.core_type(crate::stdlib::CoreTypeId::None)
                    && !definitely_returns(&function.body)
                {
                    let result = checker.type_name(signature.completion);
                    checker.error(
                        format!(
                            "function `{}` must return `{}` on every path",
                            function.name, result
                        ),
                        function.body.span,
                    );
                }
            });
        },
    );
}

fn generalize_component(checker: &mut Checker, functions: &[FunctionId]) {
    let environment_types = checker
        .declarations
        .state_fields
        .values()
        .map(|(_, ty)| *ty)
        .chain(checker.declarations.settings.values().map(|(_, ty)| *ty))
        .chain(
            checker
                .declarations
                .globals
                .values()
                .map(|binding| binding.ty),
        )
        .collect::<Vec<_>>();
    let environment = checker.inference.unbound_variables_in(environment_types);
    let mut recursive_arguments = HashMap::new();

    for function in functions {
        let signature = checker.declarations.function_signatures[function].clone();
        let generalized = checker
            .inference
            .unbound_variables_in(signature.params.iter().copied().chain([signature.result]))
            .into_iter()
            .filter(|variable| !environment.contains(variable))
            .collect::<Vec<_>>();
        recursive_arguments.insert(
            *function,
            generalized.iter().copied().map(Type::Variable).collect(),
        );
        checker
            .declarations
            .set_function_generics(*function, generalized);
    }
    checker
        .semantics
        .resolve_recursive_call_type_arguments(&recursive_arguments);
}

fn check_action_bodies(checker: &mut Checker, program: &Program) {
    let mut actions = HashSet::new();
    for action in &program.actions {
        if !actions.insert(action.kind) {
            checker.error(
                format!("duplicate `{}` action", action.kind.name()),
                action.span,
            );
            continue;
        }
        let return_ty = action_return_type(checker, program, action.kind);
        checker.with_callable_context(
            CallableContext::Action(action.kind),
            return_ty,
            FailureContext::None,
            |checker| {
                checker.scopes.clear();
                checker.scopes.push(HashMap::new());
                checker.block(&action.body, false);
            },
        );
        if action.kind == ActionKind::OnAttach
            && program
                .state
                .as_ref()
                .is_some_and(|state| !state.layouts.is_empty())
            && !layout_selection_is_terminal(checker, &action.body)
        {
            checker.error(
                "`onAttach` must return a layout on every completing path",
                action.span,
            );
        }
    }
    if program
        .state
        .as_ref()
        .is_some_and(|state| !state.layouts.is_empty())
        && !actions.contains(&ActionKind::OnAttach)
    {
        checker.error(
            "named state layouts require an `onAttach` block that returns the selected layout",
            program.state.as_ref().unwrap().span,
        );
    }
}

fn layout_selection_is_terminal(checker: &Checker, block: &crate::ast::Block) -> bool {
    block.statements.iter().any(|statement| match statement {
        crate::ast::Stmt::Return { .. } | crate::ast::Stmt::Throw { .. } => true,
        crate::ast::Stmt::If {
            then_block,
            else_block: Some(else_block),
            ..
        } => {
            layout_selection_is_terminal(checker, then_block)
                && layout_selection_is_terminal(checker, else_block)
        }
        crate::ast::Stmt::Suspend { value, .. } => checker
            .semantics
            .standard_library_item(value.id)
            .is_some_and(|item| {
                checker.standard_library.item(item).implementation
                    == crate::stdlib::Implementation::Intrinsic(
                        crate::stdlib::IntrinsicId::ProcessClosed,
                    )
            }),
        crate::ast::Stmt::Expression(crate::ast::Expr {
            kind: crate::ast::ExprKind::Suspend { value, .. },
            ..
        }) => checker
            .semantics
            .standard_library_item(value.id)
            .is_some_and(|item| {
                checker.standard_library.item(item).implementation
                    == crate::stdlib::Implementation::Intrinsic(
                        crate::stdlib::IntrinsicId::ProcessClosed,
                    )
            }),
        _ => false,
    })
}

fn action_return_type(checker: &Checker, program: &Program, action: ActionKind) -> Type {
    match action {
        ActionKind::OnDetached | ActionKind::WhileAttached => checker.core_type(CoreTypeId::None),
        ActionKind::OnAttach => program
            .state
            .as_ref()
            .and_then(|state| state.layout_enum.as_ref())
            .map_or_else(
                || checker.core_type(CoreTypeId::None),
                |enumeration| checker.enum_type(crate::types::EnumTypeId::Source(enumeration.id)),
            ),
        ActionKind::Start | ActionKind::Split | ActionKind::Reset | ActionKind::IsLoading => {
            checker.core_type(CoreTypeId::Bool)
        }
        ActionKind::GameTime => checker.standard_type(StdlibTypeId::Duration),
    }
}
