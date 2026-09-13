//! Global initializer, state-expression, function, and action body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic,
    ast::{
        ActionKind, ExprKind, FunctionId, Program, Span, StateDecl, StateField, StateSource, Stmt,
    },
    inference::Type,
    stdlib::{CoreTypeId, ItemKind, StdlibOwner, StdlibTypeId},
    visit::{self, Visitor},
};

use super::{
    Checker,
    context::{CallableContext, DebugContext, ExpressionMode, FailureContext},
    control_flow::contains_propagation,
    declarations::Binding,
};

pub(super) fn check(checker: &mut Checker, program: &Program) {
    check_global_initializers(checker, program);
    check_state_provider_configuration(checker, program);
    check_shape_conditions(checker, program);
    super::declaration_pass::collect_conditional_fields(checker, program);
    check_function_bodies(checker, program);
    infer_source_associated_types(checker, program);
    check_state_expressions(checker, program);
    check_action_bodies(checker, program);
}

/// Infers the associated types of structurally implemented capabilities from
/// the complete signatures of their required source methods.
///
/// This deliberately runs after every user function body. A method may omit
/// its result annotation, so its body is part of the evidence that determines
/// an associated type. Projections created by earlier generic callers remain
/// ordinary inference variables until these definitions are registered.
fn infer_source_associated_types(checker: &mut Checker, program: &Program) {
    let source_types = program
        .structs
        .iter()
        .map(|structure| checker.inference.type_store().id_for_struct(structure.id))
        .chain(
            program
                .enum_declarations()
                .map(|enumeration| checker.inference.type_store().id_for_enum(enumeration.id)),
        )
        .collect::<Vec<_>>();
    let capabilities = checker
        .standard_library
        .capabilities()
        .iter()
        .filter(|capability| {
            capability.behavior == crate::stdlib::CapabilityBehavior::StructuralMethods
                && !capability.associated_types.is_empty()
        })
        .map(|capability| {
            let requirements = checker
                .standard_library
                .children_of(StdlibOwner::Capability(capability.id))
                .filter_map(|symbol| match symbol {
                    crate::stdlib::StdlibSymbolId::Item(item)
                        if checker.standard_library.item(item).implementation
                            == crate::stdlib::Implementation::CapabilityRequirement =>
                    {
                        Some(*checker.standard_library.item(item))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            (*capability, requirements)
        })
        .collect::<Vec<_>>();

    for receiver in source_types {
        for (capability, requirements) in &capabilities {
            let mut associated = HashMap::new();
            let mut complete = true;
            for requirement in requirements {
                let Some(signature) = checker
                    .declarations
                    .methods
                    .get(&(Type::Known(receiver), requirement.name.to_owned()))
                    .cloned()
                else {
                    complete = false;
                    break;
                };
                let mut parameters = HashMap::new();
                let ItemKind::Method {
                    receiver: required_receiver,
                } = requirement.kind
                else {
                    unreachable!("capability requirements are methods")
                };
                let Some((&actual_receiver, actual_parameters)) = signature.params.split_first()
                else {
                    complete = false;
                    break;
                };
                if !match_capability_contract_type(
                    checker,
                    required_receiver,
                    actual_receiver,
                    &mut parameters,
                    &mut associated,
                ) || actual_parameters.len() != requirement.signature.parameters.len()
                    || !actual_parameters
                        .iter()
                        .zip(requirement.signature.parameters)
                        .all(|(&actual, required)| {
                            match_capability_contract_type(
                                checker,
                                required.ty,
                                actual,
                                &mut parameters,
                                &mut associated,
                            )
                        })
                    || !match_capability_contract_type(
                        checker,
                        requirement.signature.result,
                        signature.completion,
                        &mut parameters,
                        &mut associated,
                    )
                    || !source_method_type_parameters_match(
                        checker,
                        requirement,
                        &signature,
                        &parameters,
                    )
                {
                    complete = false;
                    break;
                }
            }
            if !complete
                || capability
                    .associated_types
                    .iter()
                    .any(|declaration| !associated.contains_key(declaration.name))
            {
                continue;
            }
            for declaration in capability.associated_types {
                let value = associated[declaration.name];
                let requirements = crate::inference::Requirements::capabilities(
                    declaration.constraints.iter().copied(),
                );
                if checker.inference.require(value, requirements).is_err()
                    || checker
                        .inference
                        .define_source_associated_type(
                            receiver,
                            capability.id,
                            declaration.name,
                            value,
                        )
                        .is_err()
                {
                    break;
                }
            }
        }
    }
}

fn source_method_type_parameters_match(
    checker: &mut Checker,
    requirement: &crate::stdlib::StdlibItem,
    signature: &crate::typeck::declarations::FunctionSignature,
    bindings: &HashMap<&'static str, Type>,
) -> bool {
    let inherited = requirement
        .signature
        .type_parameters
        .len()
        .saturating_sub(requirement.signature.explicit_type_parameters);
    requirement.signature.type_parameters[inherited..]
        .iter()
        .all(|parameter| {
            let Some(Type::Variable(variable)) = bindings
                .get(parameter.name)
                .copied()
                .map(|ty| checker.inference.shallow(ty))
            else {
                return false;
            };
            let variable = match checker.inference.shallow(Type::Variable(variable)) {
                Type::Variable(variable) => variable,
                _ => return false,
            };
            signature.generalized.contains(&variable)
                && parameter.constraints.iter().all(|required| {
                    checker.standard_library.capabilities_satisfy(
                        checker.inference.variable_requirements(variable).as_slice(),
                        *required,
                    )
                })
        })
}

fn match_capability_contract_type(
    checker: &mut Checker,
    required: crate::stdlib::TypeRef,
    actual: Type,
    parameters: &mut HashMap<&'static str, Type>,
    associated: &mut HashMap<&'static str, Type>,
) -> bool {
    use crate::stdlib::TypeRef as Required;

    let actual = checker.inference.shallow(actual);
    match required {
        Required::Core(required) => matches!(
            actual,
            Type::Known(actual)
                if matches!(
                    checker.inference.type_store().kind(actual),
                    crate::types::TypeKind::Builtin(found) if *found == required
                )
        ),
        Required::Standard(required) => matches!(
            actual,
            Type::Known(actual)
                if matches!(
                    checker.inference.type_store().kind(actual),
                    crate::types::TypeKind::Standard(found) if *found == required
                )
        ),
        Required::Parameter(name) => bind_contract_type(checker, parameters, name, actual),
        Required::Associated(name) => bind_contract_type(checker, associated, name, actual),
        Required::Async(required) => actual_async_value(checker, actual).is_some_and(|actual| {
            match_capability_contract_type(checker, *required, actual, parameters, associated)
        }),
        Required::Iterator(required) => {
            actual_iterator_item(checker, actual).is_some_and(|actual| {
                match_capability_contract_type(checker, *required, actual, parameters, associated)
            })
        }
        Required::Callable {
            parameters: required_parameters,
            result,
        } => {
            let Some((actual_parameters, actual_result)) = actual_callable(checker, actual) else {
                return false;
            };
            required_parameters.len() == actual_parameters.len()
                && required_parameters
                    .iter()
                    .zip(actual_parameters)
                    .all(|(&required, actual)| {
                        match_capability_contract_type(
                            checker, required, actual, parameters, associated,
                        )
                    })
                && match_capability_contract_type(
                    checker,
                    *result,
                    actual_result,
                    parameters,
                    associated,
                )
        }
        Required::FixedArray { element, length } => {
            let Some((actual_element, actual_length)) = actual_array(checker, actual) else {
                return false;
            };
            actual_length == Some(length)
                && match_capability_contract_type(
                    checker,
                    *element,
                    actual_element,
                    parameters,
                    associated,
                )
        }
        Required::Application {
            constructor,
            arguments,
        } => {
            let Some((actual_constructor, actual_arguments)) = actual_application(checker, actual)
            else {
                return false;
            };
            constructor == actual_constructor
                && arguments.len() == actual_arguments.len()
                && arguments
                    .iter()
                    .zip(actual_arguments)
                    .all(|(&required, actual)| {
                        match_capability_contract_type(
                            checker, required, actual, parameters, associated,
                        )
                    })
        }
    }
}

fn bind_contract_type(
    checker: &mut Checker,
    bindings: &mut HashMap<&'static str, Type>,
    name: &'static str,
    actual: Type,
) -> bool {
    match bindings.get(name).copied() {
        None => {
            bindings.insert(name, actual);
            true
        }
        Some(previous) => capability_contract_types_equal(checker, previous, actual),
    }
}

fn capability_contract_types_equal(checker: &mut Checker, left: Type, right: Type) -> bool {
    let left = checker.inference.shallow(left);
    let right = checker.inference.shallow(right);
    if left == right {
        return true;
    }
    if let (Some((left_element, left_length)), Some((right_element, right_length))) =
        (actual_array(checker, left), actual_array(checker, right))
    {
        return left_length == right_length
            && capability_contract_types_equal(checker, left_element, right_element);
    }
    if let (Some(left_value), Some(right_value)) = (
        actual_async_value(checker, left),
        actual_async_value(checker, right),
    ) {
        return capability_contract_types_equal(checker, left_value, right_value);
    }
    if let (Some((left_parameters, left_result)), Some((right_parameters, right_result))) = (
        actual_callable(checker, left),
        actual_callable(checker, right),
    ) {
        return left_parameters.len() == right_parameters.len()
            && left_parameters
                .into_iter()
                .zip(right_parameters)
                .all(|(left, right)| capability_contract_types_equal(checker, left, right))
            && capability_contract_types_equal(checker, left_result, right_result);
    }
    match (
        actual_application(checker, left),
        actual_application(checker, right),
    ) {
        (Some((left_constructor, left_arguments)), Some((right_constructor, right_arguments))) => {
            left_constructor == right_constructor
                && left_arguments.len() == right_arguments.len()
                && left_arguments
                    .into_iter()
                    .zip(right_arguments)
                    .all(|(left, right)| capability_contract_types_equal(checker, left, right))
        }
        _ => false,
    }
}

fn actual_array(checker: &mut Checker, actual: Type) -> Option<(Type, Option<u32>)> {
    match checker.inference.shallow(actual) {
        Type::Array(array) => Some((
            checker.inference.array_element(array),
            checker.inference.array_length(array),
        )),
        Type::Known(actual) => match checker.inference.type_store().kind(actual) {
            crate::types::TypeKind::Array {
                element, length, ..
            } => Some((Type::Known(*element), *length)),
            _ => None,
        },
        _ => None,
    }
}

fn actual_async_value(checker: &mut Checker, actual: Type) -> Option<Type> {
    match checker.inference.shallow(actual) {
        Type::Async(future) => Some(checker.inference.async_value(future)),
        Type::Known(actual) => match checker.inference.type_store().kind(actual) {
            crate::types::TypeKind::Async { value, .. } => Some(Type::Known(*value)),
            _ => None,
        },
        _ => None,
    }
}

fn actual_iterator_item(checker: &mut Checker, actual: Type) -> Option<Type> {
    match checker.inference.shallow(actual) {
        Type::Iterator(iterator) => Some(checker.inference.iterator_item(iterator)),
        Type::Known(actual) => match checker.inference.type_store().kind(actual) {
            crate::types::TypeKind::Iterator { item, .. } => Some(Type::Known(*item)),
            _ => None,
        },
        _ => None,
    }
}

fn actual_callable(checker: &mut Checker, actual: Type) -> Option<(Vec<Type>, Type)> {
    match checker.inference.shallow(actual) {
        Type::Callable(callable) => Some((
            checker.inference.callable_parameters(callable).to_vec(),
            checker.inference.callable_result(callable),
        )),
        Type::Known(actual) => match checker.inference.type_store().kind(actual) {
            crate::types::TypeKind::Callable {
                parameters, result, ..
            } => Some((
                parameters.iter().copied().map(Type::Known).collect(),
                Type::Known(*result),
            )),
            _ => None,
        },
        _ => None,
    }
}

fn actual_application(
    checker: &mut Checker,
    actual: Type,
) -> Option<(crate::stdlib::StdlibTypeConstructorId, Vec<Type>)> {
    let actual = checker.inference.shallow(actual);
    if let Some((element, _)) = actual_array(checker, actual) {
        return Some((crate::stdlib::StdlibTypeConstructorId::Array, vec![element]));
    }
    match actual {
        Type::Option(option) => Some((
            crate::stdlib::StdlibTypeConstructorId::Option,
            vec![checker.inference.option_value(option)],
        )),
        Type::Result(result) => Some((
            crate::stdlib::StdlibTypeConstructorId::Result,
            vec![checker.inference.result_value(result)],
        )),
        Type::Set(set) => Some((
            crate::stdlib::StdlibTypeConstructorId::Set,
            vec![checker.inference.set_element(set)],
        )),
        Type::Range(range) => Some((
            match checker.inference.range_kind(range) {
                crate::ast::RangeKind::Exclusive => {
                    crate::stdlib::StdlibTypeConstructorId::ExclusiveRange
                }
                crate::ast::RangeKind::Inclusive => {
                    crate::stdlib::StdlibTypeConstructorId::InclusiveRange
                }
            },
            vec![checker.inference.range_bound(range)],
        )),
        Type::Application(application) => Some((
            checker.inference.application_constructor(application),
            checker
                .inference
                .application_arguments(application)
                .to_vec(),
        )),
        Type::Known(actual) => match checker.inference.type_store().kind(actual) {
            crate::types::TypeKind::Option { value, .. } => Some((
                crate::stdlib::StdlibTypeConstructorId::Option,
                vec![Type::Known(*value)],
            )),
            crate::types::TypeKind::Result { value, .. } => Some((
                crate::stdlib::StdlibTypeConstructorId::Result,
                vec![Type::Known(*value)],
            )),
            crate::types::TypeKind::Set { element, .. } => Some((
                crate::stdlib::StdlibTypeConstructorId::Set,
                vec![Type::Known(*element)],
            )),
            crate::types::TypeKind::Range { bound, kind, .. } => Some((
                match kind {
                    crate::ast::RangeKind::Exclusive => {
                        crate::stdlib::StdlibTypeConstructorId::ExclusiveRange
                    }
                    crate::ast::RangeKind::Inclusive => {
                        crate::stdlib::StdlibTypeConstructorId::InclusiveRange
                    }
                },
                vec![Type::Known(*bound)],
            )),
            crate::types::TypeKind::Application {
                constructor,
                arguments,
                ..
            } => Some((
                *constructor,
                arguments.iter().copied().map(Type::Known).collect(),
            )),
            _ => None,
        },
        Type::Variable(_)
        | Type::Async(_)
        | Type::Iterator(_)
        | Type::Callable(_)
        | Type::Array(_) => None,
    }
}

fn check_shape_conditions(checker: &mut Checker, program: &Program) {
    let expected = checker.core_type(CoreTypeId::Bool);
    checker.scopes.clear();
    checker.scopes.push(HashMap::new());
    if let Some(state) = &program.state {
        for field in &state.fields {
            let Some(ty) = checker
                .declarations
                .state_fields_by_id
                .get(&field.id)
                .copied()
            else {
                continue;
            };
            checker.scopes.last_mut().unwrap().insert(
                field.name.clone(),
                Binding {
                    id: Some(field.id),
                    ty,
                    mutable: false,
                    debug_only: false,
                    declaration_span: Some(field.span),
                },
            );
        }
        for condition in state
            .conditional_fields
            .iter()
            .filter_map(|group| group.condition.as_ref())
        {
            check_shape_condition(checker, condition, expected);
        }
    }
    checker.scopes.clear();
    for condition in program
        .managed_class_declarations()
        .into_iter()
        .flat_map(|class| &class.conditional_fields)
        .filter_map(|group| group.condition.as_ref())
    {
        check_shape_condition(checker, condition, expected);
    }
}

fn check_shape_condition(
    checker: &mut Checker,
    condition: &crate::ast::Expr,
    expected: crate::inference::Type,
) {
    let mut conditional_bindings = ConditionalShapeBindingCollector::default();
    conditional_bindings.visit_expr(condition);
    for (name, span) in conditional_bindings.bindings {
        checker.error(
            format!(
                "shape predicates cannot introduce conditional binding `{}`",
                name
            ),
            span,
        );
    }
    checker.expr(condition, Some(expected));
}

#[derive(Default)]
struct ConditionalShapeBindingCollector {
    bindings: Vec<(String, Span)>,
}

impl<'ast> Visitor<'ast> for ConditionalShapeBindingCollector {
    fn visit_expr(&mut self, expression: &'ast crate::ast::Expr) {
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            pattern.kind.visit_bindings(&mut |binding| {
                self.bindings
                    .push((binding.name.clone(), binding.name_span));
            });
        }
        visit::walk_expr(self, expression);
    }
}

fn check_state_provider_configuration(checker: &mut Checker, program: &Program) {
    let Some(state) = program.state.as_ref() else {
        return;
    };
    let configurations = state
        .provider
        .iter()
        .filter_map(|provider| {
            Some((
                provider.selector.as_ref()?,
                checker.resolutions.state_provider()?,
                checker.resolutions.state_provider_selector()?,
            ))
        })
        .chain(
            state
                .provider_alternatives
                .iter()
                .filter_map(|alternative| {
                    let reference = alternative.provider.selector.as_ref()?;
                    let resolved = checker
                        .resolutions
                        .state_provider_alternative(alternative.variant)?;
                    Some((reference, resolved.provider, resolved.selector?))
                }),
        )
        .collect::<Vec<_>>();
    for (reference, provider_id, selector_index) in configurations {
        let selector = checker
            .standard_library
            .state_provider(provider_id)
            .selectors[selector_index];
        checker.scopes.clear();
        for (argument, parameter) in reference.arguments.iter().zip(selector.parameters) {
            let expected = checker.catalog_type(parameter.ty, &HashMap::new());
            let expected_name = checker.type_name(expected);
            checker.with_expected_type_source(
                super::ExpectedTypeSource {
                    span: reference.name_span,
                    label: format!(
                        "selector parameter `{}` is declared as `{expected_name}`",
                        parameter.name
                    ),
                },
                |checker| checker.expr(argument, Some(expected)),
            );
        }
    }
}

fn check_global_initializers(checker: &mut Checker, program: &Program) {
    let inferred_options = globals_inferred_as_options(program);
    for global in &program.globals {
        let inferred = if let Some(value) = &global.value {
            checker.with_debug_context(
                DebugContext::from_declaration(global.debug_only),
                |checker| {
                    let expected =
                        global
                            .annotation
                            .map(|ty| checker.syntax_type(ty))
                            .or_else(|| {
                                inferred_options.contains(&global.name).then(|| {
                                    let value = checker.fresh_inference(
                                        crate::inference::Requirements::none(),
                                        None,
                                    );
                                    Type::Option(checker.inference.option_type(value))
                                })
                            });
                    if global.annotation.is_some()
                        && let Some(expected) = expected
                    {
                        let expected_name = checker.type_name(expected);
                        let label = global.binding.simple_binding().map_or_else(
                            || format!("global binding pattern is declared as `{expected_name}`"),
                            |binding| {
                                format!(
                                    "global variable `{}` is declared as `{expected_name}`",
                                    binding.name
                                )
                            },
                        );
                        checker.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: global.name_span,
                                label,
                            },
                            |checker| checker.expr(value, Some(expected)),
                        )
                    } else {
                        checker.expr(value, expected)
                    }
                },
            )
        } else {
            Some(
                global
                    .annotation
                    .map(|ty| checker.syntax_type(ty))
                    .unwrap_or_else(|| {
                        checker.fresh_inference(crate::inference::Requirements::none(), None)
                    }),
            )
        };
        let ty = inferred.unwrap_or_else(|| checker.error_type());
        let bindings = checker.check_irrefutable_pattern(
            &global.binding,
            ty,
            global.mutable,
            global.debug_only,
            "global variable declaration",
        );
        if global.value.is_none() {
            checker.declarations.bare_globals.insert(global.id);
        }
        for (name, mut binding) in bindings {
            let mut binding_ty = binding.ty;
            let unsupported_standard =
                checker
                    .standard_type_id(binding_ty)
                    .is_some_and(|standard| {
                        standard != StdlibTypeId::String
                            && !checker
                                .standard_library
                                .type_decl(standard)
                                .value_usage
                                .global_variable
                    });
            let initialized = global.value.is_some();
            let unsupported_option = matches!(binding_ty, Type::Option(_))
                && global.binding.simple_binding().is_some()
                && !global
                    .value
                    .as_ref()
                    .is_some_and(|value| matches!(value.kind, crate::ast::ExprKind::None));
            if initialized
                && (unsupported_standard
                    || matches!(binding_ty, Type::Result(_))
                    || unsupported_option)
            {
                let ty_name = checker.type_name(binding_ty);
                checker.error(
                    format!("global variables cannot currently store `{ty_name}`"),
                    binding.declaration_span.unwrap_or(global.span),
                );
                binding_ty = checker.error_type();
                binding.ty = binding_ty;
            }
            if checker.is_provider_value_name(&name) {
                checker.error(
                    format!("`{name}` is reserved by the state provider"),
                    binding.declaration_span.unwrap_or(global.span),
                );
                continue;
            }
            if checker.declarations.globals.contains_key(&name) {
                checker.error(
                    format!("duplicate global variable `{name}`"),
                    binding.declaration_span.unwrap_or(global.span),
                );
                continue;
            }
            checker.declarations.globals.insert(name, binding);
        }
    }
}

/// Finds unannotated `None` globals whose later assignments provide the
/// contained type of an option. Global initializers are checked before bodies,
/// so this small declaration-shape pass preserves bidirectional inference
/// without treating every standalone `None` global as an ambiguous `T?`.
fn globals_inferred_as_options(program: &Program) -> HashSet<String> {
    let candidates = program
        .globals
        .iter()
        .filter(|global| {
            global.annotation.is_none()
                && global
                    .value
                    .as_ref()
                    .is_some_and(|value| matches!(value.kind, ExprKind::None))
        })
        .filter_map(|global| {
            global
                .binding
                .simple_binding()
                .map(|binding| binding.name.clone())
        })
        .collect::<HashSet<_>>();

    struct AssignmentCollector<'a> {
        candidates: &'a HashSet<String>,
        assigned: HashSet<String>,
    }

    impl<'ast> Visitor<'ast> for AssignmentCollector<'_> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::Assign {
                name,
                op: None,
                value,
                ..
            } = statement
                && !matches!(value.kind, ExprKind::None)
                && self.candidates.contains(name)
            {
                self.assigned.insert(name.clone());
            }
            visit::walk_stmt(self, statement);
        }
    }

    let mut collector = AssignmentCollector {
        candidates: &candidates,
        assigned: HashSet::new(),
    };
    collector.visit_program(program);
    collector.assigned
}

fn check_state_expressions(checker: &mut Checker, program: &Program) {
    checker.scopes.clear();
    checker.with_expression_mode(ExpressionMode::StateSource, |checker| {
        let Some(state) = program.state.as_ref() else {
            return;
        };
        for field in &state.fields {
            check_state_expression(checker, field);
        }
        for group in &state.conditional_fields {
            let predicate = group.fields.first().and_then(|field| {
                checker
                    .declarations
                    .conditional_state_field_predicates
                    .get(&field.id)
                    .cloned()
            });
            checker.with_shape_predicate(predicate.as_ref(), |checker| {
                for field in &group.fields {
                    check_state_expression(checker, field);
                }
            });
        }
        for alternative in &state.provider_alternatives {
            checker.with_provider_variant(Some(alternative.variant), |checker| {
                for field in &alternative.fields {
                    check_state_expression(checker, field);
                }
            });
        }
        check_state_dependency_cycles(checker, state);
    });
}

/// Checks one state-field source in the shape context established by its
/// declaration. The caller owns that context so conditional state fields and
/// provider alternatives can refine every expression attached to the field uniformly.
fn check_state_expression(checker: &mut Checker, field: &StateField) {
    checker.with_state_field(field.id, |checker| {
        check_state_expression_inner(checker, field)
    });
}

fn check_state_expression_inner(checker: &mut Checker, field: &StateField) {
    let field_type = checker.declarations.state_fields_by_id[&field.id];
    if let StateSource::Expression(expression) = &field.source {
        let boundary = contains_propagation(expression)
            .then(|| Type::Result(checker.inference.result_type(field_type)));
        let contextual_poll_result = super::expressions::expression_is_bare_none(expression)
            .then(|| Type::Result(checker.inference.result_type(field_type)));
        let (actual, failure) = checker.with_failure_context(
            boundary.map_or(FailureContext::None, FailureContext::boundary),
            |checker| {
                if contextual_poll_result.is_some() {
                    let field_type_name = checker.type_name(field_type);
                    checker.with_expected_type_source(
                        super::ExpectedTypeSource {
                            span: state_field_declaration_span(field),
                            label: format!(
                                "state field `{}` is declared as `{field_type_name}`",
                                field.name
                            ),
                        },
                        |checker| checker.expr(expression, contextual_poll_result),
                    )
                } else {
                    checker.expr(expression, None)
                }
            },
        );
        let used_propagation = failure.propagated();
        if let Some(actual) = actual {
            let actual = checker.shallow_type(actual);
            let poll_result = if used_propagation {
                let boundary = boundary.expect("propagation syntax creates a failure boundary");
                if let Type::Result(result) = actual {
                    let value = checker.inference.result_value(result);
                    unify_state_field_value(checker, value, field_type, field, expression.span);
                    checker.expect_expression(
                        expression.id,
                        actual,
                        Some(boundary),
                        expression.span,
                    );
                } else {
                    unify_state_field_value(checker, actual, field_type, field, expression.span);
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
                unify_state_field_value(checker, value, field_type, field, expression.span);
                actual
            } else {
                unify_state_field_value(checker, actual, field_type, field, expression.span);
                let result = Type::Result(checker.inference.result_type(actual));
                checker.expect_expression(expression.id, actual, Some(result), expression.span);
                result
            };
            checker
                .semantics
                .resolve_state_poll_result(field.id, poll_result);
        }
    }

    if let StateSource::Pointer(path) = &field.source
        && let crate::ast::PointerPathBase::Expression(base) = &path.base
    {
        let expected = state_pointer_base_type(checker);
        checker.with_expected_type_source(
            super::ExpectedTypeSource {
                span: base.span,
                label: "a dynamic `at` base must be an address-valued sibling state field"
                    .to_owned(),
            },
            |checker| {
                checker.expr(base, Some(expected));
            },
        );
        if !matches!(
            checker.semantics.resolved_value(base.id),
            Some(crate::semantic::ResolvedValue::StateCandidate(_))
        ) {
            checker.error(
                "a dynamic `at` base must start from a sibling state field",
                base.span,
            );
        }
    }

    if let Some(transform) = &field.transform {
        checker.scopes.push(HashMap::from([(
            "value".to_owned(),
            Binding {
                id: Some(transform.value),
                ty: field_type,
                mutable: false,
                debug_only: false,
                declaration_span: Some(field.span),
            },
        )]));
        checker
            .semantics
            .resolve_value_type(transform.value, field_type);
        let poll_result = Type::Result(checker.inference.result_type(field_type));
        let field_type_name = checker.type_name(field_type);
        let (actual, _) =
            checker.with_failure_context(FailureContext::boundary(poll_result), |checker| {
                checker.with_expected_type_source(
                    super::ExpectedTypeSource {
                        span: state_field_declaration_span(field),
                        label: format!(
                            "state field `{}` is declared as `{field_type_name}`",
                            field.name
                        ),
                    },
                    |checker| checker.expr(&transform.expression, Some(poll_result)),
                )
            });
        if actual.is_none() {
            checker.error(
                "a state field filter must produce a value or an error",
                transform.expression.span,
            );
        }
        checker.scopes.pop();
    }
}

fn state_pointer_base_type(checker: &mut Checker) -> Type {
    let Some(provider) = checker.active_state_provider() else {
        return checker.core_type(crate::stdlib::CoreTypeId::Address);
    };
    let provider = checker.standard_library.state_provider(provider);
    let parameter = checker
        .standard_library
        .item(provider.direct_read)
        .signature
        .parameters[0]
        .ty;
    checker.catalog_type(parameter, &std::collections::HashMap::new())
}

fn check_state_dependency_cycles(checker: &mut Checker, state: &StateDecl) {
    use std::collections::{HashMap, HashSet};

    let fields = state.all_fields().collect::<Vec<_>>();
    let positions = fields
        .iter()
        .enumerate()
        .map(|(position, field)| (field.id, position))
        .collect::<HashMap<_, _>>();
    let mut index = 0usize;
    let mut indices = HashMap::new();
    let mut lowlinks = HashMap::new();
    let mut stack = Vec::new();
    let mut on_stack = HashSet::new();
    let mut components = Vec::<Vec<crate::ast::ValueId>>::new();

    struct Tarjan<'a> {
        checker: &'a Checker,
        positions: &'a HashMap<crate::ast::ValueId, usize>,
        index: &'a mut usize,
        indices: &'a mut HashMap<crate::ast::ValueId, usize>,
        lowlinks: &'a mut HashMap<crate::ast::ValueId, usize>,
        stack: &'a mut Vec<crate::ast::ValueId>,
        on_stack: &'a mut HashSet<crate::ast::ValueId>,
        components: &'a mut Vec<Vec<crate::ast::ValueId>>,
    }

    impl Tarjan<'_> {
        fn visit(&mut self, field: crate::ast::ValueId) {
            let current = *self.index;
            *self.index += 1;
            self.indices.insert(field, current);
            self.lowlinks.insert(field, current);
            self.stack.push(field);
            self.on_stack.insert(field);

            for dependency in self.checker.semantics.state_dependencies(field) {
                if !self.positions.contains_key(dependency) {
                    continue;
                }
                if !self.indices.contains_key(dependency) {
                    self.visit(*dependency);
                    let dependency_low = self.lowlinks[dependency];
                    self.lowlinks
                        .entry(field)
                        .and_modify(|low| *low = (*low).min(dependency_low));
                } else if self.on_stack.contains(dependency) {
                    let dependency_index = self.indices[dependency];
                    self.lowlinks
                        .entry(field)
                        .and_modify(|low| *low = (*low).min(dependency_index));
                }
            }

            if self.lowlinks[&field] != current {
                return;
            }
            let mut component = Vec::new();
            loop {
                let member = self.stack.pop().expect("a component root remains on stack");
                self.on_stack.remove(&member);
                component.push(member);
                if member == field {
                    break;
                }
            }
            component.sort_by_key(|field| self.positions[field]);
            self.components.push(component);
        }
    }

    {
        let mut tarjan = Tarjan {
            checker,
            positions: &positions,
            index: &mut index,
            indices: &mut indices,
            lowlinks: &mut lowlinks,
            stack: &mut stack,
            on_stack: &mut on_stack,
            components: &mut components,
        };
        for field in &fields {
            if !tarjan.indices.contains_key(&field.id) {
                tarjan.visit(field.id);
            }
        }
    }

    for component in components {
        let cyclic = component.len() > 1
            || checker
                .semantics
                .state_dependencies(component[0])
                .contains(&component[0]);
        if !cyclic {
            continue;
        }
        let first = fields[positions[&component[0]]];
        let mut diagnostic = crate::Diagnostic::type_error(
            "state fields cannot depend on each other cyclically",
            first.span,
        )
        .with_primary_label(format!("`{}` participates in this cycle", first.name));
        for member in component.iter().skip(1) {
            let field = fields[positions[member]];
            diagnostic = diagnostic.with_secondary_label(
                field.span,
                format!("`{}` also participates in this cycle", field.name),
            );
        }
        checker.errors.push(diagnostic);
    }
}

fn unify_state_field_value(
    checker: &mut Checker,
    actual: Type,
    expected: Type,
    field: &crate::ast::StateField,
    span: Span,
) {
    if field.annotation.is_some() {
        let expected_name = checker.type_name(expected);
        checker.with_expected_type_source(
            super::ExpectedTypeSource {
                span: state_field_declaration_span(field),
                label: format!(
                    "state field `{}` is declared as `{expected_name}`",
                    field.name
                ),
            },
            |checker| {
                checker.unify_expected(actual, expected, span);
            },
        );
    } else {
        checker.unify(actual, expected, span);
    }
}

fn state_field_declaration_span(field: &crate::ast::StateField) -> Span {
    let end = match &field.source {
        StateSource::Expression(expression) => expression.span.start,
        StateSource::Pointer(path) => path.at_span.map_or(field.span.end, |span| span.start),
    };
    Span {
        start: field.span.start,
        end,
    }
}

fn check_function_bodies(checker: &mut Checker, program: &Program) {
    // Recovered declarations can leave gaps in parser-assigned IDs.
    let functions = program
        .functions
        .iter()
        .map(|function| (function.id, function))
        .collect::<std::collections::HashMap<_, _>>();
    for component in super::function_graph::dependency_order(program) {
        checker.active_function_component = component.functions.iter().copied().collect();
        for function_id in &component.functions {
            let function = functions
                .get(function_id)
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
            let generator_item = crate::typeck::control_flow::contains_yield(&function.body)
                .then(|| actual_iterator_item(checker, signature.result))
                .flatten();
            let library_item = checker
                .standard_library
                .source_body_item_by_function_name(&function.name);
            if let Some(item) = library_item {
                seed_library_body_signature(checker, *item, &signature, function.span);
            }
            let failure = match checker.shallow_type(signature.completion) {
                result @ Type::Result(_) => FailureContext::boundary(result),
                _ => FailureContext::None,
            };
            let callable = library_item
                .map(|item| CallableContext::LibraryFunction(item.id))
                .unwrap_or_else(|| {
                    if function
                        .name
                        .starts_with(crate::stdlib::RESERVED_FUNCTION_PREFIX)
                    {
                        CallableContext::CompilerGenerated
                    } else {
                        CallableContext::Function
                    }
                });
            let return_type_source = function.return_annotation_span.map(|span| {
                let result = checker.type_name(signature.completion);
                super::ExpectedTypeSource {
                    span,
                    label: format!(
                        "function `{}` is declared to return `{result}`",
                        function.name
                    ),
                }
            });
            checker.with_return_type_source(return_type_source, |checker| {
                checker.with_callable_context(callable, signature.completion, failure, |checker| {
                    checker.with_generator_item(generator_item, |checker| {
                        checker.scopes.clear();
                        checker.scopes.push(HashMap::new());
                        for (parameter, ty) in
                            function.params.iter().zip(signature.params.iter().copied())
                        {
                            checker.bind_irrefutable_parameter(
                                &parameter.binding,
                                ty,
                                checker.debug_context.is_debug(),
                                "function parameter",
                                "function",
                            );
                        }
                        checker.block(&function.body, false);
                        if generator_item.is_none()
                            && signature.completion
                                != checker.core_type(crate::stdlib::CoreTypeId::None)
                            && !block_is_terminal(checker, &function.body)
                        {
                        let result = checker.type_name(signature.completion);
                        let tail = function.body.statements.last().and_then(|statement| {
                            match statement {
                                Stmt::Expression(expression) => Some(expression),
                                _ => None,
                            }
                        });
                        let mut diagnostic = if let Some(tail) = tail {
                            Diagnostic::type_error(
                                "functions do not implicitly return their final expression",
                                tail.span,
                            )
                            .with_primary_label(format!(
                                "this `{result}` value is currently discarded"
                            ))
                            .with_note(
                                "add `return` in a function body; only nested value blocks use their final expression as a value",
                            )
                            .with_machine_applicable_fix(
                                "return the final expression",
                                Span {
                                    start: tail.span.start,
                                    end: tail.span.start,
                                },
                                "return ",
                            )
                        } else {
                            Diagnostic::type_error(
                                format!(
                                    "function `{}` must return `{}` on every path",
                                    function.name, result
                                ),
                                function.body.span,
                            )
                            .with_primary_label("this body can reach its end without returning")
                        };
                        if let Some(source) = checker.return_type_source.clone() {
                            diagnostic = diagnostic.with_secondary_label(source.span, source.label);
                        }
                            checker.errors.push(diagnostic);
                        }
                    });
                });
            });
        },
    );
}

fn seed_library_body_signature(
    checker: &mut Checker,
    item: crate::stdlib::StdlibItem,
    inferred: &crate::typeck::declarations::FunctionSignature,
    span: Span,
) {
    let mut variables = item
        .signature
        .type_parameters
        .iter()
        .map(|parameter| {
            let requirements = parameter.constraints.iter().fold(
                crate::inference::Requirements::none(),
                |requirements, constraint| {
                    requirements | crate::inference::Requirements::capability(*constraint)
                },
            );
            (parameter.name, checker.fresh_inference(requirements, None))
        })
        .collect::<HashMap<_, _>>();
    if let StdlibOwner::Capability(capability) = item.owner {
        let receiver = match item.kind {
            ItemKind::Method {
                receiver: crate::stdlib::TypeRef::Parameter(name),
            } => variables[name],
            ItemKind::Method { receiver } => checker.catalog_type(receiver, &variables),
            ItemKind::Function => unreachable!("capability members are receiver methods"),
            ItemKind::Constant => unreachable!("capabilities do not declare constants"),
        };
        for (owner, associated) in checker
            .standard_library
            .capability_associated_types(capability)
        {
            let value = checker
                .inference
                .associated_type(receiver, owner, associated.name);
            variables.insert(associated.name, value);
        }
    } else if let StdlibOwner::TypeConstructor(constructor) = item.owner
        && matches!(item.kind, ItemKind::Method { .. })
    {
        // Associated types depend on the concrete owner parameters. Static
        // constructors such as `Map.new<A, B>` are ordinary functions and do
        // not have an instantiated owner whose associated types could be
        // projected while checking their generic body.
        for associated in checker
            .standard_library
            .type_constructor(constructor)
            .associated_types
        {
            let value = checker.catalog_type(associated.value, &variables);
            variables.insert(associated.name, value);
        }
    }

    let mut declared_parameters = Vec::new();
    if let ItemKind::Method { receiver } = item.kind {
        declared_parameters.push(checker.catalog_type(receiver, &variables));
    }
    declared_parameters.extend(
        item.signature
            .parameters
            .iter()
            .map(|parameter| checker.catalog_type(parameter.ty, &variables)),
    );
    for (actual, declared) in inferred.params.iter().copied().zip(declared_parameters) {
        checker.unify(actual, declared, span);
    }
    let result = checker.catalog_type(item.signature.result, &variables);
    // Async catalog signatures describe the value produced on completion,
    // while synchronous signatures describe the callable's direct result.
    // A generator is synchronous at the call boundary: its direct result is
    // the iterator frame and its body completion is `None`.
    let actual_result = if item.signature.result_is_async {
        inferred.completion
    } else {
        inferred.result
    };
    checker.unify(actual_result, result, span);
}

fn generalize_component(checker: &mut Checker, functions: &[FunctionId]) {
    let environment_types = checker
        .declarations
        .state_fields_by_id
        .values()
        .copied()
        .chain(checker.declarations.settings.values().map(|(_, ty)| *ty))
        .chain(
            checker
                .declarations
                .globals
                .values()
                .map(|binding| binding.ty),
        )
        .collect::<Vec<_>>();
    let environment = checker
        .inference
        .unbound_variables_in(environment_types.iter().copied());
    let environment_array_shapes = checker
        .inference
        .unbound_array_shapes_in(environment_types.iter().copied());
    let mut recursive_arguments = HashMap::new();

    for function in functions {
        let signature = checker.declarations.function_signatures[function].clone();
        let mut generalized = checker
            .inference
            .unbound_variables_in(signature.params.iter().copied().chain([signature.result]))
            .into_iter()
            .filter(|variable| !environment.contains(variable))
            .collect::<Vec<_>>();
        let generalized_array_shapes = checker
            .inference
            .unbound_array_shapes_in(signature.params.iter().copied().chain([signature.result]))
            .into_iter()
            .filter(|shape| !environment_array_shapes.contains(shape))
            .collect();
        let mut associated_projections = Vec::new();
        loop {
            let projections = checker.inference.associated_projections_for(&generalized);
            let mut changed = false;
            for projection in projections {
                if !associated_projections.contains(&projection) {
                    associated_projections.push(projection);
                }
                for output in checker
                    .inference
                    .unbound_variables_in([Type::Variable(projection.output)])
                {
                    if !generalized.contains(&output) && !environment.contains(&output) {
                        generalized.push(output);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        recursive_arguments.insert(
            *function,
            generalized.iter().copied().map(Type::Variable).collect(),
        );
        checker.declarations.set_function_generics(
            *function,
            generalized,
            generalized_array_shapes,
            associated_projections,
        );
    }
    checker
        .semantics
        .resolve_recursive_call_type_arguments(&recursive_arguments);
}

fn check_action_bodies(checker: &mut Checker, program: &Program) {
    if let Some((assigned, missing)) = crate::shape_selection::partial_shape_selection(program)
        && let Some(action) = program
            .actions
            .iter()
            .find(|action| action.kind == ActionKind::OnAttach)
    {
        checker.errors.push(
            Diagnostic::type_error(
                "`onAttach` cannot mix explicit and provider-inferred attachment shape",
                action.span,
            )
            .with_primary_label(
                "assign every attachment-shape global here, or let provider metadata initialize all of them",
            )
            .with_note(format!("assigned here: {}", assigned.join(", ")))
            .with_note(format!("still provider-inferred: {}", missing.join(", "))),
        );
    }
    let explicit_attachment_shape = crate::shape_selection::has_explicit_shape_selection(program);
    let automatic_selection = automatic_shape_selection(checker, program);
    if !explicit_attachment_shape
        && let crate::shape_selection::AutomaticShapeSelection::RequiresExplicit(reason) =
            &automatic_selection
    {
        let span = program
            .state
            .as_ref()
            .map_or(crate::ast::Span::default(), |state| state.span);
        checker.errors.push(
            Diagnostic::type_error(
                "attachment-shape globals require explicit initialization",
                span,
            )
            .with_primary_label("initialize every shape global in `onAttach`")
            .with_note(reason.note()),
        );
    }
    let mut actions = HashSet::new();
    for action in &program.actions {
        if !actions.insert(action.kind) {
            checker.error(
                format!("duplicate `{}` action", action.kind.name()),
                action.span,
            );
            continue;
        }
        let return_ty = action_return_type(checker, action.kind);
        checker
            .semantics
            .resolve_action_result(action.kind, return_ty);
        let failure = match action.kind {
            ActionKind::SelectProcess => FailureContext::boundary(return_ty),
            ActionKind::OnAttach => {
                let boundary = Type::Result(checker.inference.result_type(return_ty));
                FailureContext::boundary(boundary)
            }
            _ => FailureContext::None,
        };
        checker.with_callable_context(
            CallableContext::Action(action.kind),
            return_ty,
            failure,
            |checker| {
                checker.scopes.clear();
                checker.scopes.push(HashMap::new());
                checker.block(&action.body, false);
            },
        );
    }
}

fn automatic_shape_selection(
    checker: &mut Checker,
    program: &Program,
) -> crate::shape_selection::AutomaticShapeSelection {
    let mut enum_by_dimension = HashMap::new();
    for dimension in checker.shape_dimensions.clone() {
        let crate::typeck::declarations::ShapeDimension::Global(value) = dimension else {
            continue;
        };
        let Some(ty) = checker
            .declarations
            .globals
            .values()
            .find_map(|binding| (binding.id == Some(value)).then_some(binding.ty))
        else {
            continue;
        };
        let Type::Known(ty) = checker.shallow_type(ty) else {
            continue;
        };
        if let crate::types::TypeKind::Enum(enumeration) = checker.inference.type_store().kind(ty) {
            enum_by_dimension.insert(
                crate::semantic::ResolvedShapeDimension::Global(value),
                *enumeration,
            );
        }
    }
    let selection = crate::shape_selection::automatic_shape_selection_with(
        program,
        |field| enum_by_dimension.get(&field).copied(),
        |field| {
            checker
                .declarations
                .conditional_managed_fields
                .get(&field)
                .map(|predicate| {
                    predicate
                        .alternatives
                        .iter()
                        .map(|alternative| {
                            alternative
                                .iter()
                                .map(|constraint| {
                                    let dimension = match constraint.dimension {
                                        crate::typeck::declarations::ShapeDimension::Global(
                                            value,
                                        ) => crate::semantic::ResolvedShapeDimension::Global(value),
                                        crate::typeck::declarations::ShapeDimension::StateField(
                                            value,
                                        ) => crate::semantic::ResolvedShapeDimension::StateField(
                                            value,
                                        ),
                                    };
                                    (dimension, constraint.variant)
                                })
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default()
        },
    );
    if let crate::shape_selection::AutomaticShapeSelection::Available(plan) = &selection
        && !plan.evidence_fields.is_empty()
        && checker.resolutions.state_provider() != Some(crate::stdlib::StdlibStateProviderId::Unity)
    {
        crate::shape_selection::AutomaticShapeSelection::RequiresExplicit(
            crate::shape_selection::ShapeSelectionReason::EvidenceUnavailable,
        )
    } else {
        selection
    }
}

pub(super) fn block_is_terminal(checker: &mut Checker, block: &crate::ast::Block) -> bool {
    block
        .statements
        .iter()
        .any(|statement| statement_is_terminal(checker, statement))
}

pub(super) fn statement_is_terminal(checker: &mut Checker, statement: &crate::ast::Stmt) -> bool {
    match statement {
        // A debug statement is removed from release builds, so control flow
        // outside it must remain valid without relying on its body diverging.
        crate::ast::Stmt::Debug { .. } => false,
        crate::ast::Stmt::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            expression_is_never(checker, condition)
                || else_block.as_ref().is_some_and(|else_block| {
                    block_is_terminal(checker, then_block) && block_is_terminal(checker, else_block)
                })
        }
        crate::ast::Stmt::Variable(variable) => expression_is_never(
            checker,
            variable
                .value
                .as_ref()
                .expect("local variables have initializers"),
        ),
        crate::ast::Stmt::Assign { value, .. }
        | crate::ast::Stmt::StateAssign { value, .. }
        | crate::ast::Stmt::IndexAssign { value, .. } => expression_is_never(checker, value),
        crate::ast::Stmt::While { condition, .. } => expression_is_never(checker, condition),
        crate::ast::Stmt::For { iterable, .. } => expression_is_never(checker, iterable),
        crate::ast::Stmt::Suspend { returns: true, .. } => true,
        crate::ast::Stmt::Suspend { mode, value, .. } => {
            let Some(mut ty) = checker.semantics.inferred_expression_type(value.id) else {
                return false;
            };
            ty = checker.shallow_type(ty);
            let completion = match (mode, ty) {
                (crate::ast::SuspensionMode::Await, Type::Async(future)) => {
                    checker.inference.async_value(future)
                }
                (crate::ast::SuspensionMode::Await, Type::Result(result))
                | (crate::ast::SuspensionMode::Retry, Type::Result(result)) => {
                    checker.inference.result_value(result)
                }
                _ => ty,
            };
            checker.is_never_type(completion)
        }
        // `yield` transfers control out of the current `next()` call, but the
        // generator resumes at the following statement. It therefore is not
        // terminal for lexical fallthrough or value-block typing.
        crate::ast::Stmt::Yield { .. } => false,
        crate::ast::Stmt::Expression(expression) => expression_is_never(checker, expression),
    }
}

fn expression_is_never(checker: &mut Checker, expression: &crate::ast::Expr) -> bool {
    checker
        .semantics
        .inferred_expression_type(expression.id)
        .is_some_and(|ty| checker.is_never_type(ty))
}

fn action_return_type(checker: &mut Checker, action: ActionKind) -> Type {
    match action {
        ActionKind::SelectProcess => {
            let boolean = checker.core_type(CoreTypeId::Bool);
            Type::Result(checker.inference.result_type(boolean))
        }
        ActionKind::Setup
        | ActionKind::OnDetach
        | ActionKind::OnStateReady
        | ActionKind::OnStart
        | ActionKind::OnReset => checker.core_type(CoreTypeId::None),
        ActionKind::OnAttach => checker.core_type(CoreTypeId::None),
        ActionKind::WhileAttached
        | ActionKind::Start
        | ActionKind::Split
        | ActionKind::Reset
        | ActionKind::IsLoading => checker.core_type(CoreTypeId::Bool),
        ActionKind::GameTime => checker.standard_type(StdlibTypeId::Duration),
    }
}
