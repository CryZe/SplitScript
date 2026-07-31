//! Global initializer, state-expression, function, and action body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{Program, StateSource},
    inference::Type,
};

use super::{
    Checker,
    context::{CallableContext, DebugContext, ExpressionMode, FailureContext},
    control_flow::{contains_propagation, definitely_returns, is_constant},
    declarations::Binding,
};

pub(super) fn check(checker: &mut Checker, program: &Program) {
    check_global_initializers(checker, program);
    check_state_expressions(checker, program);
    check_function_bodies(checker, program);
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
        if !is_constant(&global.value) {
            checker.error(
                "global initializers must be None, numeric, boolean, or payload-free enum constants",
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
                !checker
                    .standard_library
                    .type_decl(standard)
                    .value_usage
                    .global_variable
            });
            if unsupported_standard
                || ty == checker.core_type(crate::stdlib::CoreTypeId::Void)
                || matches!(ty, Type::Array(_) | Type::Result(_))
                || matches!(ty, Type::Option(_))
                    && !matches!(global.value.kind, crate::ast::ExprKind::None)
                || checker.source_record_id(ty).is_some()
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
        for field in &program.state.as_ref().unwrap().fields {
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
    for function in &program.functions {
        checker.with_debug_context(
            DebugContext::from_declaration(function.debug_only),
            |checker| {
                let signature = checker.declarations.function_signatures[&function.id].clone();
                let failure = match checker.shallow_type(signature.result) {
                    result @ Type::Result(_) => FailureContext::boundary(result),
                    _ => FailureContext::None,
                };
                let callable = CallableContext::Function(function.method_of.map_or_else(
                    || format!("function `{}`", function.name),
                    |receiver| format!("method `{receiver}.{}`", function.name),
                ));
                checker.with_callable_context(callable, signature.result, failure, |checker| {
                    checker.scopes.clear();
                    checker.scopes.push(HashMap::new());
                    for (parameter, ty) in
                        function.params.iter().zip(signature.params.iter().copied())
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
                    if signature.result != checker.core_type(crate::stdlib::CoreTypeId::Void)
                        && !definitely_returns(&function.body)
                    {
                        let result = checker.type_name(signature.result);
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
        let return_ty = checker.syntax_type(action.kind.return_type());
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
    }
}
