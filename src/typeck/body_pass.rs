//! Global initializer, state-expression, function, and action body checking.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic, DiagnosticFix, FixApplicability, TextEdit,
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
    check_layout_conditions(checker, program);
    check_function_bodies(checker, program);
    check_state_expressions(checker, program);
    check_action_bodies(checker, program);
}

fn check_layout_conditions(checker: &mut Checker, program: &Program) {
    checker.scopes.clear();
    let expected = checker.core_type(CoreTypeId::Bool);
    for condition in program
        .state
        .iter()
        .flat_map(|state| &state.conditional_fields)
        .filter_map(|group| group.condition.as_ref())
        .chain(
            program
                .managed_class_declarations()
                .into_iter()
                .flat_map(|class| &class.conditional_fields)
                .filter_map(|group| group.condition.as_ref()),
        )
    {
        checker.expr(condition, Some(expected));
    }
}

fn check_state_provider_configuration(checker: &mut Checker, program: &Program) {
    let Some(reference) = program
        .state
        .as_ref()
        .and_then(|state| state.provider.as_ref())
        .and_then(|provider| provider.selector.as_ref())
    else {
        return;
    };
    let Some(provider_id) = checker.resolutions.state_provider() else {
        return;
    };
    let Some(selector_index) = checker.resolutions.state_provider_selector() else {
        return;
    };
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

fn check_global_initializers(checker: &mut Checker, program: &Program) {
    let inferred_options = globals_inferred_as_options(program);
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
                        checker.with_expected_type_source(
                            super::ExpectedTypeSource {
                                span: global.name_span,
                                label: format!(
                                    "global variable `{}` is declared as `{expected_name}`",
                                    global.name,
                                ),
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
        let mut ty = inferred.unwrap_or_else(|| checker.error_type());
        let unsupported_standard = checker.standard_type_id(ty).is_some_and(|standard| {
            standard != StdlibTypeId::String
                && !checker
                    .standard_library
                    .type_decl(standard)
                    .value_usage
                    .global_variable
        });
        if let Some(value) = &global.value
            && (unsupported_standard
                || matches!(ty, Type::Result(_))
                || matches!(ty, Type::Option(_))
                    && !matches!(value.kind, crate::ast::ExprKind::None))
        {
            let name = checker.type_name(ty);
            checker.error(
                format!("global variables cannot currently store `{name}`"),
                global.span,
            );
            ty = checker.error_type();
        }
        checker.semantics.resolve_value_type(global.id, ty);
        if global.value.is_none() {
            checker.declarations.bare_globals.insert(global.id);
        }
        checker.declarations.globals.insert(
            global.name.clone(),
            Binding {
                id: Some(global.id),
                ty,
                mutable: global.mutable,
                debug_only: global.debug_only,
                declaration_span: Some(global.name_span),
            },
        );
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
        .map(|global| global.name.clone())
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
            checker.with_layout_predicate(predicate.as_ref(), |checker| {
                for field in &group.fields {
                    check_state_expression(checker, field);
                }
            });
        }
        for layout in &state.layouts {
            checker.with_state_layout(Some(layout.variant), |checker| {
                for field in &layout.fields {
                    check_state_expression(checker, field);
                }
            });
        }
        check_state_dependency_cycles(checker, state);
    });
}

/// Checks one state-field source in the layout context established by its
/// declaration. The caller owns that context so conditional state fields and
/// named layouts can refine every expression attached to the field uniformly.
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
        let (actual, failure) = checker.with_failure_context(
            boundary.map_or(FailureContext::None, FailureContext::boundary),
            |checker| checker.expr(expression, None),
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
    let Some((provider, _)) = checker.provider_value else {
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
            if let Some(item) = checker
                .standard_library
                .all_items()
                .iter()
                .find(|item| match item.implementation {
                    crate::stdlib::Implementation::LibraryBody { function_name, .. } => {
                        function_name == function.name
                    }
                    crate::stdlib::Implementation::LibraryOverloads { cases, .. } => cases
                        .iter()
                        .any(|case| case.function_name == function.name),
                    _ => false,
                })
                .copied()
            {
                seed_library_body_signature(checker, item, &signature, function.span);
            }
            let failure = match checker.shallow_type(signature.completion) {
                result @ Type::Result(_) => FailureContext::boundary(result),
                _ => FailureContext::None,
            };
            let callable = checker
                .standard_library
                .all_items()
                .iter()
                .find_map(|item| match item.implementation {
                    crate::stdlib::Implementation::LibraryBody { function_name, .. }
                        if function_name == function.name =>
                    {
                        Some(CallableContext::LibraryFunction(item.id))
                    }
                    crate::stdlib::Implementation::LibraryOverloads { cases, .. }
                        if cases
                            .iter()
                            .any(|case| case.function_name == function.name) =>
                    {
                        Some(CallableContext::LibraryFunction(item.id))
                    }
                    _ => None,
                })
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
                                    declaration_span: Some(parameter.span),
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
        for associated in checker
            .standard_library
            .capability(capability)
            .associated_types
        {
            let value = checker
                .inference
                .associated_type(receiver, capability, associated.name);
            variables.insert(associated.name, value);
        }
    } else if let StdlibOwner::TypeConstructor(constructor) = item.owner {
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
    checker.unify(inferred.completion, result, span);
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
    let environment = checker.inference.unbound_variables_in(environment_types);
    let mut recursive_arguments = HashMap::new();

    for function in functions {
        let signature = checker.declarations.function_signatures[function].clone();
        let mut generalized = checker
            .inference
            .unbound_variables_in(signature.params.iter().copied().chain([signature.result]))
            .into_iter()
            .filter(|variable| !environment.contains(variable))
            .collect::<Vec<_>>();
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
        checker
            .declarations
            .set_function_generics(*function, generalized, associated_projections);
    }
    checker
        .semantics
        .resolve_recursive_call_type_arguments(&recursive_arguments);
}

fn check_action_bodies(checker: &mut Checker, program: &Program) {
    let explicit_attachment_layout = crate::layout_selection::has_explicit_layout_return(program);
    let automatic_attachment_layout = !explicit_attachment_layout
        && matches!(
            automatic_layout_selection(checker, program),
            crate::layout_selection::AutomaticLayoutSelection::Available(_)
        );
    checker.layout_available_in_on_attach = automatic_attachment_layout;
    let mut actions = HashSet::new();
    for action in &program.actions {
        if !actions.insert(action.kind) {
            checker.error(
                format!("duplicate `{}` action", action.kind.name()),
                action.span,
            );
            continue;
        }
        let return_ty =
            action_return_type(checker, program, action.kind, automatic_attachment_layout);
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
            && program.state.as_ref().is_some_and(|state| {
                !state.layouts.is_empty()
                    || (state.layout.is_some() && !automatic_attachment_layout)
            })
            && !block_is_terminal(checker, &action.body)
        {
            let mut diagnostic = Diagnostic::type_error(
                "`onAttach` must return a layout on every completing path",
                action.span,
            )
            .with_primary_label("this selector can finish without choosing a layout");
            if program
                .state
                .as_ref()
                .is_some_and(|state| state.provider.is_none())
            {
                let insertion = action.body.span.end.saturating_sub(1);
                diagnostic = diagnostic
                    .with_note(
                        "keep an unsupported build attached but inert by awaiting `process.closed()` instead of selecting a fallback layout",
                    )
                    .with_machine_applicable_fix(
                        "wait for an unsupported process to close",
                        Span {
                            start: insertion,
                            end: insertion,
                        },
                        "\n    await process.closed()\n",
                    );
            } else {
                diagnostic = diagnostic.with_note(
                    "return a layout on every completing path; this state provider has no generic process-close wait",
                );
            }
            checker.errors.push(diagnostic);
        }
    }
    if let Some(state) = program.state.as_ref() {
        let missing_named = !state.layouts.is_empty() && !actions.contains(&ActionKind::OnAttach);
        let missing_dimensions =
            state.layout.is_some() && !automatic_attachment_layout && !explicit_attachment_layout;
        if missing_named || missing_dimensions {
            let selection = automatic_layout_selection(checker, program);
            checker
                .errors
                .push(missing_layout_selector_diagnostic(state, &selection));
        }
    }
}

fn automatic_layout_selection(
    checker: &mut Checker,
    program: &Program,
) -> crate::layout_selection::AutomaticLayoutSelection {
    let mut enum_by_dimension = HashMap::new();
    if let Some(layout) = program
        .state
        .as_ref()
        .and_then(|state| state.layout.as_ref())
    {
        let record = &program.records[layout.record.index()];
        for field in &record.fields {
            let ty = checker.syntax_type(field.ty);
            let Type::Known(ty) = checker.shallow_type(ty) else {
                continue;
            };
            if let crate::types::TypeKind::Enum(enumeration) =
                checker.inference.type_store().kind(ty)
            {
                enum_by_dimension.insert(field.id, *enumeration);
            }
        }
    }
    let selection = crate::layout_selection::automatic_layout_selection_with(
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
                                .map(|constraint| (constraint.dimension, constraint.variant))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default()
        },
    );
    if let crate::layout_selection::AutomaticLayoutSelection::Available(plan) = &selection
        && !plan.evidence_fields.is_empty()
        && checker.resolutions.state_provider() != Some(crate::stdlib::StdlibStateProviderId::Unity)
    {
        crate::layout_selection::AutomaticLayoutSelection::RequiresExplicit(
            crate::layout_selection::ExplicitSelectionReason::EvidenceUnavailable,
        )
    } else {
        selection
    }
}

fn missing_layout_selector_diagnostic(
    state: &StateDecl,
    selection: &crate::layout_selection::AutomaticLayoutSelection,
) -> Diagnostic {
    if state.layout.is_some() {
        let mut diagnostic = Diagnostic::type_error(
            "layout dimensions require an `onAttach` block that returns the selected `Layout`",
            state.span,
        )
        .with_primary_label("these dimensions need an explicit attach-time value");
        if let crate::layout_selection::AutomaticLayoutSelection::RequiresExplicit(reason) =
            selection
        {
            diagnostic = diagnostic.with_note(reason.note());
        }
        return diagnostic;
    }
    let diagnostic = Diagnostic::type_error(
        "named state layouts require an `onAttach` block that returns the selected layout",
        state.span,
    )
    .with_primary_label("these layouts need an explicit attach-time selector");
    if state.provider.is_some() {
        return diagnostic.with_note(
            "return a layout only after identifying the supported target; this state provider has no generic process-close wait",
        );
    }
    let variants = state
        .layout_enum
        .as_ref()
        .expect("named layouts have a generated enum")
        .variants
        .iter()
        .map(|variant| {
            format!(
                "    // if <{} build check> {{\n    //     return StateLayout.{}\n    // }}\n",
                variant.name, variant.name
            )
        })
        .collect::<String>();
    let fix = DiagnosticFix {
        title: "add a safe `onAttach` layout-selection skeleton".to_owned(),
        applicability: FixApplicability::HasPlaceholders,
        edits: vec![TextEdit {
            span: Span {
                start: state.span.end,
                end: state.span.end,
            },
            replacement: format!("\n\nonAttach {{\n{variants}    await process.closed()\n}}"),
        }],
    };
    diagnostic
        .with_note(
            "select only builds identified by reliable process or module evidence; leave every unknown build at `await process.closed()`",
        )
        .with_fix(fix)
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
        crate::ast::Stmt::Expression(expression) => expression_is_never(checker, expression),
    }
}

fn expression_is_never(checker: &mut Checker, expression: &crate::ast::Expr) -> bool {
    checker
        .semantics
        .inferred_expression_type(expression.id)
        .is_some_and(|ty| checker.is_never_type(ty))
}

fn action_return_type(
    checker: &Checker,
    program: &Program,
    action: ActionKind,
    automatic_attachment_layout: bool,
) -> Type {
    match action {
        ActionKind::Setup
        | ActionKind::OnDetach
        | ActionKind::OnStateReady
        | ActionKind::OnStart
        | ActionKind::OnReset => checker.core_type(CoreTypeId::None),
        ActionKind::OnAttach => program.state.as_ref().map_or_else(
            || checker.core_type(CoreTypeId::None),
            |state| {
                if automatic_attachment_layout {
                    checker.core_type(CoreTypeId::None)
                } else if let Some(layout) = &state.layout {
                    checker.record_type(layout.record)
                } else if let Some(enumeration) = &state.layout_enum {
                    checker.enum_type(crate::types::EnumTypeId::Source(enumeration.id))
                } else {
                    checker.core_type(CoreTypeId::None)
                }
            },
        ),
        ActionKind::WhileAttached
        | ActionKind::Start
        | ActionKind::Split
        | ActionKind::Reset
        | ActionKind::IsLoading => checker.core_type(CoreTypeId::Bool),
        ActionKind::GameTime => checker.standard_type(StdlibTypeId::Duration),
    }
}
