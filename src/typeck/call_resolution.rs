//! Call, path, catalog-overload, and member resolution during type checking.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic,
    ast::{ActionKind, ArrayTypeId, Expr, ExprId, ExprKind, Span},
    inference::{InferenceError, Requirements, Type, type_may_have_capability},
    migration::{
        ASL_SETTINGS_ADD_DIAGNOSTIC, ForeignSpellingContext, foreign_spelling,
        legacy_array_field_diagnostic, legacy_set_field_diagnostic, legacy_static_call_diagnostic,
        legacy_string_field_diagnostic, legacy_string_method_diagnostic,
        legacy_value_path_diagnostic, migration_diagnostic,
    },
    semantic::{PendingResolvedCall, ResolvedMember, ResolvedValue},
    signature::parse_signature,
    stdlib::{
        Availability, DeclaredTypeRef, ItemKind, ParameterRule, StandardBinaryOperator,
        StandardUnaryOperator, StdlibItem, StdlibItemId, StdlibOwner, StdlibTypeConstructorId,
        StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    stdlib_semantic::{CallCandidate, StandardLibrarySemanticExt},
    types::TypeKind,
};

use super::{
    CallSyntax, Checker, DeferredMemberPath, MethodReceiver, PathResolution, ResolvedReceiver,
    catalog_type_argument, closest_name,
    context::{CallableContext, ExpressionMode},
};

impl Checker {
    /// Resolves unary syntax through a catalog-declared zero-argument method.
    pub(super) fn resolve_unary_operator(
        &mut self,
        op: crate::ast::UnaryOp,
        operand_type: Type,
        expression: ExprId,
        operand: ExprId,
        span: Span,
    ) -> Option<Type> {
        let operator = match op {
            crate::ast::UnaryOp::Not => StandardUnaryOperator::Not,
            crate::ast::UnaryOp::Neg => StandardUnaryOperator::Negate,
        };
        let candidates = self
            .standard_library
            .unary_operator_candidates(operator)
            .into_iter()
            .collect();
        let error_count = self.errors.len();
        let resolved = self.operator_call(
            candidates,
            operand_type,
            None,
            ResolvedReceiver::Expression {
                expression: operand,
                members: Vec::new(),
            },
            span,
            false,
        );
        let Some((result, call)) = resolved else {
            if self.errors.len() == error_count {
                let operand = self.type_name(operand_type);
                let requirement = match op {
                    crate::ast::UnaryOp::Not => "`bool` or an integer",
                    crate::ast::UnaryOp::Neg => "a signed number",
                };
                self.error(
                    format!("operator cannot be applied to `{operand}`; expected {requirement}"),
                    span,
                );
            }
            return None;
        };
        self.semantics.resolve_call(expression, call);
        Some(result)
    }

    /// Resolves binary syntax through a catalog-declared method. Inferred
    /// operands select capability bindings; concrete operands may select a
    /// more specific implementation owned by their exact type.
    pub(super) fn resolve_binary_operator(
        &mut self,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        expression: ExprId,
        left: ExprId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Expression {
                expression: left,
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_call(expression, call);
        Some(result)
    }

    pub(super) fn resolve_assignment_operator(
        &mut self,
        assignment: crate::ast::AssignmentId,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        target: crate::ast::ValueId,
        span: Span,
    ) -> Option<Type> {
        let (result, call) = self.binary_operator_call(
            op,
            left_type,
            right_type,
            ResolvedReceiver::Path {
                root: ResolvedValue::Variable(target),
                members: Vec::new(),
            },
            span,
        )?;
        self.semantics.resolve_assignment_call(assignment, call);
        Some(result)
    }

    fn binary_operator_call(
        &mut self,
        op: crate::ast::BinaryOp,
        left_type: Type,
        right_type: Type,
        receiver: ResolvedReceiver,
        span: Span,
    ) -> Option<(Type, PendingResolvedCall)> {
        let operator = match op {
            crate::ast::BinaryOp::Add => StandardBinaryOperator::Add,
            crate::ast::BinaryOp::Sub => StandardBinaryOperator::Subtract,
            crate::ast::BinaryOp::Mul => StandardBinaryOperator::Multiply,
            crate::ast::BinaryOp::Div => StandardBinaryOperator::Divide,
            crate::ast::BinaryOp::Rem => StandardBinaryOperator::Remainder,
            crate::ast::BinaryOp::BitOr => StandardBinaryOperator::BitOr,
            crate::ast::BinaryOp::BitXor => StandardBinaryOperator::BitXor,
            crate::ast::BinaryOp::BitAnd => StandardBinaryOperator::BitAnd,
            crate::ast::BinaryOp::Shl => StandardBinaryOperator::ShiftLeft,
            crate::ast::BinaryOp::Shr => StandardBinaryOperator::ShiftRight,
            crate::ast::BinaryOp::Eq => StandardBinaryOperator::Equal,
            crate::ast::BinaryOp::Ne => StandardBinaryOperator::NotEqual,
            crate::ast::BinaryOp::Lt => StandardBinaryOperator::LessThan,
            crate::ast::BinaryOp::Le => StandardBinaryOperator::LessThanOrEqual,
            crate::ast::BinaryOp::Gt => StandardBinaryOperator::GreaterThan,
            crate::ast::BinaryOp::Ge => StandardBinaryOperator::GreaterThanOrEqual,
            _ => return None,
        };
        let candidates = self
            .standard_library
            .binary_operator_candidates(operator)
            .into_iter()
            .collect();
        self.operator_call(
            candidates,
            left_type,
            Some(right_type),
            receiver,
            span,
            true,
        )
    }

    fn operator_call(
        &mut self,
        candidates: Vec<CallCandidate>,
        receiver_operand: Type,
        argument_operand: Option<Type>,
        receiver: ResolvedReceiver,
        span: Span,
        generic_only_when_inferred: bool,
    ) -> Option<(Type, PendingResolvedCall)> {
        let inferred_receiver = matches!(self.shallow_type(receiver_operand), Type::Variable(_));
        let candidates = candidates
            .into_iter()
            .filter(|candidate| {
                (!generic_only_when_inferred
                    || !inferred_receiver
                    || matches!(candidate.receiver(), Some(CatalogTypeRef::Parameter(_))))
                    && self.inferred_receiver_may_select(candidate, receiver_operand)
                    && self.catalog_candidate_may_apply(candidate, receiver_operand)
            })
            .collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else {
            if candidates.len() > 1 {
                self.error(
                    format!(
                        "operator is ambiguous between {}",
                        candidates
                            .iter()
                            .map(|candidate| candidate.item.qualified_name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    span,
                );
            }
            return None;
        };
        let item = candidate.item;
        let mut variables = HashMap::new();
        for parameter in item.signature.type_parameters {
            let requirements = parameter
                .constraints
                .iter()
                .fold(Requirements::none(), |requirements, constraint| {
                    requirements | Requirements::capability(*constraint)
                });
            let ty = self.fresh_inference(requirements.clone(), None);
            if !requirements.is_empty() {
                self.require(ty, requirements, span)?;
            }
            variables.insert(parameter.name, ty);
        }
        let receiver_type = self.catalog_type(
            candidate
                .receiver()
                .expect("operator bindings are validated methods"),
            &variables,
        );
        self.unify(receiver_operand, receiver_type, span)?;
        let mut signature = vec![receiver_type];
        match (argument_operand, item.signature.parameters) {
            (Some(argument_operand), [parameter]) => {
                let parameter_type = self.catalog_type(parameter.ty, &variables);
                self.unify(argument_operand, parameter_type, span)?;
                signature.push(parameter_type);
            }
            (None, []) => {}
            _ => unreachable!("operator bindings have a validated arity"),
        };
        let result = self.catalog_type(item.signature.result, &variables);
        signature.push(result);
        let type_arguments = item
            .signature
            .type_parameters
            .iter()
            .map(|parameter| variables[parameter.name])
            .collect();
        Some((
            result,
            PendingResolvedCall::StandardLibrary {
                item: item.id,
                type_arguments,
                signature,
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
            },
        ))
    }

    fn inferred_receiver_may_select(&mut self, candidate: &CallCandidate, receiver: Type) -> bool {
        let Type::Variable(variable) = self.shallow_type(receiver) else {
            return true;
        };
        let requirements = self.inference.variable_requirements(variable);
        let concrete = match candidate.receiver() {
            Some(CatalogTypeRef::Core(core)) => {
                Some(self.declared_type(DeclaredTypeRef::Core(core)))
            }
            Some(CatalogTypeRef::Standard(standard)) => Some(self.standard_type(standard)),
            _ => None,
        };
        concrete.is_none_or(|concrete| {
            requirements.as_slice().iter().all(|capability| {
                type_may_have_capability(
                    &self.standard_library,
                    self.inference.type_store(),
                    concrete,
                    *capability,
                )
            })
        })
    }

    pub(super) fn call(
        &mut self,
        call: CallSyntax<'_>,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let CallSyntax {
            callee,
            name_span,
            postfix_receiver,
            type_arguments,
        } = call;
        if !type_arguments.is_empty()
            && postfix_receiver.is_none()
            && matches!(callee, [name] if matches!(name.as_str(), "Some" | "Ok" | "Err"))
        {
            self.error(
                format!("`{}` does not accept explicit type arguments", callee[0]),
                span,
            );
            return None;
        }
        if postfix_receiver.is_none() && callee == ["Some"] {
            if args.len() != 1 {
                self.error("`Some` expects one value", span);
                return None;
            }
            let expected_option = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Option(option) => Some(option),
                _ => None,
            });
            if expected.is_some()
                && expected_option.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Some` constructs an optional value, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_option.map(|option| self.inference.option_value(option));
            let value = self.expr(&args[0], value_hint)?;
            let option = expected_option.unwrap_or_else(|| self.inference.option_type(value));
            let ty = Type::Option(option);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::OptionSome { option });
            return self.expect_expression(expression, ty, expected, span);
        }

        if postfix_receiver.is_none() && callee == ["Ok"] {
            if args.len() != 1 {
                self.error("`Ok` expects one value", span);
                return None;
            }
            let expected_result = expected.and_then(|ty| match self.shallow_type(ty) {
                Type::Result(result) => Some(result),
                _ => None,
            });
            if expected.is_some()
                && expected_result.is_none()
                && !matches!(
                    expected.map(|ty| self.shallow_type(ty)),
                    Some(Type::Variable(_))
                )
            {
                let other = expected.map(|ty| self.shallow_type(ty)).unwrap();
                self.error(
                    format!("`Ok` constructs a result, but `{other}` was expected"),
                    span,
                );
                return None;
            }
            let value_hint = expected_result.map(|result| self.inference.result_value(result));
            let value = self.expr(&args[0], value_hint)?;
            let result = expected_result.unwrap_or_else(|| self.inference.result_type(value));
            let ty = Type::Result(result);
            self.semantics
                .resolve_call(expression, PendingResolvedCall::ResultSuccess { result });
            return self.expect_expression(expression, ty, expected, span);
        }

        if postfix_receiver.is_none() && callee == ["Err"] {
            if args.len() != 1 {
                self.error("`Err` expects one error message", span);
                return None;
            }
            self.expr(&args[0], Some(self.standard_type(StdlibTypeId::String)));
            let result = match expected.map(|ty| self.shallow_type(ty)) {
                Some(result @ Type::Result(_)) => result,
                Some(Type::Variable(_)) | None => {
                    self.error(
                        "cannot infer the success type of `Err`; add a `T!` annotation",
                        span,
                    );
                    return None;
                }
                Some(other) => {
                    self.error(
                        format!("`Err` constructs a result, but `{other}` was expected"),
                        span,
                    );
                    return None;
                }
            };
            let Type::Result(result_id) = result else {
                unreachable!()
            };
            self.semantics.resolve_call(
                expression,
                PendingResolvedCall::ResultError { result: result_id },
            );
            return Some(result);
        }

        if let Some((active_provider, _)) = self.provider_value
            && !matches!(self.callable, CallableContext::LibraryFunction(_))
            && let Some(native_provider) = self.standard_library.source_state_provider()
            && active_provider != native_provider.id
            && callee
                .first()
                .is_some_and(|root| root == native_provider.value_name)
        {
            let provider = self.standard_library.state_provider(active_provider);
            self.error(
                format!(
                    "`{}` is unavailable under `state {}`; use `{}` instead",
                    native_provider.value_name, provider.name, provider.value_name
                ),
                span,
            );
            return None;
        }

        if let Some(receiver) = postfix_receiver {
            return self.postfix_method_call(
                receiver,
                callee,
                name_span,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }

        let standard_library = self.standard_library.clone();
        let mut function_candidates = standard_library.function_candidates(callee);
        if function_candidates.len() > 1 {
            self.ambiguous_catalog_call(callee, &function_candidates, span);
            return None;
        }
        if let Some(candidate) = function_candidates.pop() {
            return self.catalog_call(
                &candidate,
                None,
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        if callee
            .first()
            .is_some_and(|receiver| self.binding(receiver).is_none())
            && let Some(id) = legacy_static_call_diagnostic(callee)
        {
            let metadata =
                migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label);
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return None;
        }
        let (display_name, signature_id, signature_result, parameters, resolved_call) =
            if let [name] = callee {
                if !type_arguments.is_empty() {
                    self.error(
                        "explicit type arguments are currently supported on standard-library calls",
                        span,
                    );
                    return None;
                }
                let Some(signature) = self.declarations.functions.get(name).cloned() else {
                    let suggestion = self.function_name_suggestion(callee);
                    self.unknown_function(callee, name_span, span, suggestion.as_deref());
                    return None;
                };
                let signature = if self.active_function_component.contains(&signature.id) {
                    signature.monomorphic_call()
                } else {
                    signature.instantiate(&mut self.inference)
                };
                let concrete_signature = signature
                    .params
                    .iter()
                    .copied()
                    .chain([signature.result])
                    .collect();
                (
                    name.clone(),
                    signature.id,
                    signature.result,
                    signature.params,
                    PendingResolvedCall::UserFunction {
                        function: signature.id,
                        type_arguments: signature.type_arguments,
                        signature: concrete_signature,
                    },
                )
            } else {
                if let Some(suggestion) = self.function_name_suggestion(callee) {
                    self.unknown_function(callee, name_span, span, Some(&suggestion));
                    return None;
                }
                let (method, receiver_path) = callee.split_last().unwrap();
                let receiver = self.path(receiver_path, span, None)?;
                if self.is_error_type(receiver.ty) {
                    for argument in args {
                        self.expr(argument, None);
                    }
                    return self.expect_expression(expression, receiver.ty, expected, span);
                }
                let receiver_value = receiver
                    .value
                    .expect("method receiver paths resolve to a declaration or snapshot value");
                let receiver_members = receiver
                    .members
                    .expect("method receiver types must be known while resolving a call");
                let receiver_type = self.shallow_type(receiver.ty);
                let mut candidates = standard_library
                    .method_candidates(method)
                    .into_iter()
                    .filter(|candidate| self.catalog_candidate_may_apply(candidate, receiver_type))
                    .collect::<Vec<_>>();
                self.prefer_specific_catalog_candidates(&mut candidates, receiver_type);
                if candidates.len() > 1 {
                    self.ambiguous_catalog_call(callee, &candidates, span);
                    return None;
                }
                if let Some(candidate) = candidates.pop() {
                    return self.catalog_call(
                        &candidate,
                        Some(MethodReceiver {
                            ty: receiver_type,
                            value: ResolvedReceiver::Path {
                                root: receiver_value,
                                members: receiver_members,
                            },
                        }),
                        type_arguments,
                        args,
                        expected,
                        expression,
                        span,
                    );
                }
                if !type_arguments.is_empty() {
                    self.error(
                        "explicit type arguments are currently supported on standard-library calls",
                        span,
                    );
                    return None;
                }
                let Some(signature) = self
                    .declarations
                    .methods
                    .get(&(receiver_type, method.clone()))
                    .cloned()
                else {
                    let suggestion = self.method_name_suggestion(receiver_type, method);
                    self.unknown_method(
                        receiver_type,
                        method,
                        name_span,
                        span,
                        suggestion.as_deref(),
                    );
                    return None;
                };
                let signature = if self.active_function_component.contains(&signature.id) {
                    signature.monomorphic_call()
                } else {
                    signature.instantiate(&mut self.inference)
                };
                let receiver_name = self.type_name(receiver_type);
                let concrete_signature = signature
                    .params
                    .iter()
                    .copied()
                    .chain([signature.result])
                    .collect();
                (
                    format!("{receiver_name}.{method}"),
                    signature.id,
                    signature.result,
                    signature.params.into_iter().skip(1).collect(),
                    PendingResolvedCall::UserMethod {
                        function: signature.id,
                        type_arguments: signature.type_arguments,
                        signature: concrete_signature,
                        receiver: ResolvedReceiver::Path {
                            root: receiver_value,
                            members: receiver_members,
                        },
                        receiver_type,
                    },
                )
            };
        if self.declarations.debug_functions.contains(&signature_id)
            && !self.debug_context.is_debug()
        {
            self.error(
                format!("debug-only function `{display_name}` can only be called from debug code"),
                span,
            );
        }
        if args.len() != parameters.len() {
            self.error(
                format!(
                    "`{display_name}` expects {} arguments, found {}",
                    parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in args.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        let result = self.expect_expression(expression, signature_result, expected, span)?;
        self.semantics.resolve_call(expression, resolved_call);
        Some(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn postfix_method_call(
        &mut self,
        written_receiver: &Expr,
        callee: &[String],
        name_span: Span,
        type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let standard_library = self.standard_library.clone();
        let base_type = self.expr(written_receiver, None)?;
        let base_type = self.shallow_type(base_type);
        if self.is_error_type(base_type) {
            for argument in args {
                self.expr(argument, None);
            }
            return self.expect_expression(expression, base_type, expected, span);
        }
        let method = callee.last().expect("postfix calls name a method");
        let (receiver_type, receiver_members) =
            self.resolve_members(base_type, &callee[..callee.len() - 1], span)?;
        let method_receiver = MethodReceiver {
            ty: receiver_type,
            value: ResolvedReceiver::Expression {
                expression: written_receiver.id,
                members: receiver_members,
            },
        };
        let mut candidates = standard_library
            .method_candidates(method)
            .into_iter()
            .filter(|candidate| self.catalog_candidate_may_apply(candidate, receiver_type))
            .collect::<Vec<_>>();
        self.prefer_specific_catalog_candidates(&mut candidates, receiver_type);
        if candidates.len() > 1 {
            self.ambiguous_catalog_call(std::slice::from_ref(method), &candidates, span);
            return None;
        }
        if let Some(candidate) = candidates.pop() {
            return self.catalog_call(
                &candidate,
                Some(method_receiver),
                type_arguments,
                args,
                expected,
                expression,
                span,
            );
        }
        if !type_arguments.is_empty() {
            self.error(
                "explicit type arguments are currently supported on standard-library calls",
                span,
            );
            return None;
        }
        let Some(signature) = self
            .declarations
            .methods
            .get(&(receiver_type, method.to_owned()))
            .cloned()
        else {
            let suggestion = self.method_name_suggestion(receiver_type, method);
            self.unknown_method(
                receiver_type,
                method,
                name_span,
                span,
                suggestion.as_deref(),
            );
            return None;
        };
        let signature = if self.active_function_component.contains(&signature.id) {
            signature.monomorphic_call()
        } else {
            signature.instantiate(&mut self.inference)
        };
        if self.declarations.debug_functions.contains(&signature.id)
            && !self.debug_context.is_debug()
        {
            self.error(
                format!("debug-only method `{method}` can only be called from debug code"),
                span,
            );
        }
        let parameters = signature.params.iter().copied().skip(1).collect::<Vec<_>>();
        if args.len() != parameters.len() {
            self.error(
                format!(
                    "`{method}` expects {} arguments, found {}",
                    parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in args.iter().zip(parameters) {
            self.expr(argument, Some(parameter));
        }
        let concrete_signature = signature
            .params
            .iter()
            .copied()
            .chain([signature.result])
            .collect();
        let result = self.expect_expression(expression, signature.result, expected, span)?;
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::UserMethod {
                function: signature.id,
                type_arguments: signature.type_arguments,
                signature: concrete_signature,
                receiver: method_receiver.value,
                receiver_type,
            },
        );
        Some(result)
    }

    pub(super) fn function_name_suggestion(&self, callee: &[String]) -> Option<String> {
        let (name, prefix) = callee.split_last()?;
        let standard_library = self.standard_library.clone();
        let mut candidates = standard_library
            .items()
            .iter()
            .filter_map(|item| {
                let path = standard_library.item_path(item)?;
                (path.len() == callee.len()
                    && path[..path.len() - 1]
                        .iter()
                        .copied()
                        .eq(prefix.iter().map(String::as_str)))
                .then_some(item.name)
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if prefix.is_empty() {
            candidates.extend(self.declarations.functions.keys().cloned());
            candidates.extend(["Some".to_owned(), "Ok".to_owned(), "Err".to_owned()]);
        }
        closest_name(name, candidates.iter().map(String::as_str))
    }

    pub(super) fn method_name_suggestion(
        &mut self,
        receiver: Type,
        method: &str,
    ) -> Option<String> {
        let standard_library = self.standard_library.clone();
        let mut candidates = Vec::new();
        for item in standard_library.items() {
            let ItemKind::Method { .. } = item.kind else {
                continue;
            };
            let candidate = CallCandidate { item };
            if self.catalog_candidate_may_apply(&candidate, receiver) {
                candidates.push(item.name.to_owned());
            }
        }
        candidates.extend(
            self.declarations
                .methods
                .keys()
                .filter(|(candidate_receiver, _)| *candidate_receiver == receiver)
                .map(|(_, name)| name.clone()),
        );
        if let Some(migration) = foreign_spelling(method, ForeignSpellingContext::Method)
            && candidates
                .iter()
                .any(|candidate| candidate == migration.replacement.text())
        {
            return Some(migration.replacement.text().to_owned());
        }
        closest_name(method, candidates.iter().map(String::as_str))
    }

    pub(super) fn unknown_function(
        &mut self,
        callee: &[String],
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        let name = callee.join(".");
        let Some(suggestion) = suggestion else {
            self.error(format!("unknown function `{name}`"), span);
            return;
        };
        let suggested_name = callee[..callee.len() - 1]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(suggestion))
            .collect::<Vec<_>>()
            .join(".");
        self.errors.push(
            Diagnostic::type_error(
                format!("unknown function `{name}`; did you mean `{suggested_name}`?"),
                name_span,
            )
            .with_primary_label("this name is not defined")
            .with_machine_applicable_fix(
                format!("replace `{}` with `{suggestion}`", callee.last().unwrap()),
                name_span,
                suggestion,
            ),
        );
    }

    pub(super) fn unknown_method(
        &mut self,
        receiver: Type,
        method: &str,
        name_span: Span,
        span: Span,
        suggestion: Option<&str>,
    ) {
        if matches!(
            receiver,
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::Standard(StdlibTypeId::String)
                )
        ) && let Some(id) = legacy_string_method_diagnostic(method)
        {
            let metadata =
                migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label);
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return;
        }
        if method == "Add"
            && matches!(
                receiver,
                Type::Known(id)
                    if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
            )
        {
            let metadata = migration_diagnostic(ASL_SETTINGS_ADD_DIAGNOSTIC)
                .expect("type checker migration diagnostic IDs must exist");
            let mut diagnostic = Diagnostic::type_error(metadata.message, name_span)
                .with_primary_label(metadata.primary_label);
            for note in metadata.notes {
                diagnostic = diagnostic.with_note(*note);
            }
            self.errors.push(diagnostic);
            return;
        }
        let receiver = self.type_name(receiver);
        let Some(suggestion) = suggestion else {
            self.error(format!("type `{receiver}` has no method `{method}`"), span);
            return;
        };
        self.errors.push(
            Diagnostic::type_error(
                format!("type `{receiver}` has no method `{method}`; did you mean `{suggestion}`?"),
                name_span,
            )
            .with_primary_label("this method is not defined for the receiver type")
            .with_machine_applicable_fix(
                format!("replace `{method}` with `{suggestion}`"),
                name_span,
                suggestion,
            ),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn catalog_call(
        &mut self,
        candidate: &CallCandidate,
        receiver: Option<MethodReceiver>,
        explicit_type_arguments: &[crate::ast::TypeRef],
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let item = candidate.item;
        if !explicit_type_arguments.is_empty()
            && explicit_type_arguments.len() != item.signature.explicit_type_parameters
        {
            self.error(
                format!(
                    "`{}` expects {} type arguments, found {}",
                    item.qualified_name,
                    item.signature.explicit_type_parameters,
                    explicit_type_arguments.len()
                ),
                span,
            );
            return None;
        }
        let mut variables = HashMap::new();
        for (index, parameter) in item.signature.type_parameters.iter().enumerate() {
            let requirements = parameter
                .constraints
                .iter()
                .fold(Requirements::none(), |requirements, constraint| {
                    requirements | Requirements::capability(*constraint)
                });
            let explicitly_selected = index < item.signature.explicit_type_parameters
                && explicit_type_arguments.get(index).is_some();
            let ty = (index < item.signature.explicit_type_parameters)
                .then(|| explicit_type_arguments.get(index))
                .flatten()
                .map(|ty| self.syntax_type(*ty))
                .unwrap_or_else(|| self.fresh_inference(requirements.clone(), None));
            if !requirements.is_empty() {
                if explicitly_selected {
                    if self.inference.require(ty, requirements).is_err() {
                        let required = parameter
                            .constraints
                            .iter()
                            .map(|capability| self.standard_library.capability(*capability).name)
                            .collect::<Vec<_>>()
                            .join(" + ");
                        let selected = self.type_name(ty);
                        self.error(
                            format!(
                                "type `{selected}` does not satisfy the required `{required}` capability"
                            ),
                            span,
                        );
                        return None;
                    }
                } else {
                    self.require(ty, requirements, span)?;
                }
            }
            variables.insert(parameter.name, ty);
        }
        if item.id == StdlibItemId::ProcessRead && explicit_type_arguments.is_empty() {
            self.inferred_process_reads.push((variables["T"], span));
        }
        if item.id == StdlibItemId::SetNew && explicit_type_arguments.is_empty() {
            self.inferred_empty_collections
                .push((variables["U"], span, "set"));
        }
        let mut concrete_signature = Vec::new();
        if let Some(receiver) = &receiver {
            let declared_receiver = self.catalog_type(
                candidate
                    .receiver()
                    .expect("method candidates declare a receiver"),
                &variables,
            );
            self.unify(receiver.ty, declared_receiver, span)?;
            if item.id == StdlibItemId::ArrayPush
                && let Type::Array(array) = self.shallow_type(receiver.ty)
                && let Some(length) = self.inference.array_length(array)
            {
                self.error(
                    format!(
                        "cannot change the length of fixed array `[T; {length}]`; `push` is only available on growable `[T]`"
                    ),
                    span,
                );
                return None;
            }
            concrete_signature.push(declared_receiver);
        }
        let operation = self.standard_library.operation_semantics(item.id);
        let source_body_suspends = match item.implementation {
            crate::stdlib::Implementation::LibraryBody { function_name, .. } => {
                let result = self
                    .declarations
                    .functions
                    .get(function_name)
                    .map(|signature| signature.result);
                result.is_some_and(|result| matches!(self.shallow_type(result), Type::Async(_)))
            }
            crate::stdlib::Implementation::Intrinsic(_) => false,
        };
        let expected_result = expected.map(|ty| self.shallow_type(ty));
        let expected_completion = expected_result.map(|expected| match expected {
            Type::Async(future) => self.inference.async_value(future),
            expected => expected,
        });
        let completion_type = if let (Some(value), Some(Type::Result(result))) = (
            catalog_type_argument(item.signature.result, StdlibTypeConstructorId::Result),
            expected_completion,
        ) {
            let declared_value = self.catalog_type(value, &variables);
            let expected_value = self.inference.result_value(result);
            self.unify(declared_value, expected_value, span)?;
            Type::Result(result)
        } else {
            self.catalog_type(item.signature.result, &variables)
        };
        let result_type = if operation.suspension == crate::stdlib::SuspensionKind::Suspends
            || source_body_suspends
        {
            Type::Async(self.inference.async_type(completion_type))
        } else {
            completion_type
        };
        let result = self.expect_expression(expression, result_type, expected, span)?;
        if args.len() != item.signature.parameters.len() {
            self.error(
                format!(
                    "`{}` expects {} arguments, found {}",
                    item.qualified_name,
                    item.signature.parameters.len(),
                    args.len()
                ),
                span,
            );
            return None;
        }
        for (argument, parameter) in args.iter().zip(item.signature.parameters) {
            let parameter_type = self.catalog_type(parameter.ty, &variables);
            self.expr(argument, Some(parameter_type));
            self.validate_catalog_argument(argument, parameter.rule, item);
            concrete_signature.push(parameter_type);
        }
        concrete_signature.push(result_type);
        if operation.availability == Availability::OnAttach && !self.callable.can_suspend() {
            self.error(
                format!("`{}` must be awaited in `onAttach`", item.qualified_name),
                span,
            );
        }
        let type_arguments = item
            .signature
            .type_parameters
            .iter()
            .map(|parameter| variables[parameter.name])
            .collect();
        self.semantics.resolve_call(
            expression,
            PendingResolvedCall::StandardLibrary {
                item: item.id,
                type_arguments,
                signature: concrete_signature,
                receiver: receiver.as_ref().map(|receiver| receiver.value.clone()),
                receiver_type: receiver.as_ref().map(|receiver| receiver.ty),
            },
        );
        Some(result)
    }

    pub(super) fn catalog_candidate_may_apply(
        &mut self,
        candidate: &CallCandidate,
        receiver: Type,
    ) -> bool {
        let receiver = self.shallow_type(receiver);
        let declared = candidate
            .receiver()
            .expect("only method candidates are matched against receivers");
        match declared {
            CatalogTypeRef::Core(expected) => {
                let expected = self.declared_type(DeclaredTypeRef::Core(expected));
                matches!(receiver, Type::Variable(_)) || receiver == expected
            }
            CatalogTypeRef::Standard(standard) => {
                matches!(receiver, Type::Variable(_)) || receiver == self.standard_type(standard)
            }
            CatalogTypeRef::Application { constructor, .. } => {
                matches!(receiver, Type::Variable(_))
                    || (constructor == StdlibTypeConstructorId::Array
                        && matches!(receiver, Type::Array(_)))
                    || (constructor == StdlibTypeConstructorId::Option
                        && matches!(receiver, Type::Option(_)))
                    || (constructor == StdlibTypeConstructorId::Result
                        && matches!(receiver, Type::Result(_)))
                    || (constructor == StdlibTypeConstructorId::Set
                        && matches!(receiver, Type::Set(_)))
            }
            CatalogTypeRef::FixedArray { length, .. } => {
                matches!(receiver, Type::Variable(_))
                    || matches!(receiver, Type::Array(array) if self.inference.array_length(array) == Some(length))
            }
            CatalogTypeRef::Parameter(name) => candidate
                .item
                .signature
                .type_parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .is_none_or(|parameter| {
                    parameter.constraints.iter().all(|constraint| {
                        matches!(receiver, Type::Variable(_))
                            || type_may_have_capability(
                                &self.standard_library,
                                self.inference.type_store(),
                                receiver,
                                *constraint,
                            )
                    })
                }),
        }
    }

    fn prefer_specific_catalog_candidates(
        &mut self,
        candidates: &mut Vec<CallCandidate>,
        receiver: Type,
    ) {
        if candidates.len() < 2 {
            return;
        }
        let receiver_is_inferred = matches!(self.shallow_type(receiver), Type::Variable(_));
        if receiver_is_inferred && let CallableContext::LibraryFunction(item) = self.callable {
            let owner = self.standard_library.item(item).owner;
            if candidates
                .iter()
                .any(|candidate| candidate.item.owner == owner)
            {
                candidates.retain(|candidate| candidate.item.owner == owner);
            }
        }
        let specificity = |candidate: &CallCandidate| match candidate.receiver() {
            Some(CatalogTypeRef::Parameter(_)) => 1,
            Some(_) if receiver_is_inferred => 0,
            Some(_) => 2,
            None => 0,
        };
        let strongest = candidates.iter().map(specificity).max().unwrap_or_default();
        candidates.retain(|candidate| specificity(candidate) == strongest);
    }

    pub(super) fn ambiguous_catalog_call(
        &mut self,
        callee: &[String],
        candidates: &[CallCandidate],
        span: Span,
    ) {
        let names = candidates
            .iter()
            .map(|candidate| candidate.item.qualified_name)
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            format!(
                "call to `{}` is ambiguous between {names}",
                callee.join(".")
            ),
            span,
        );
    }

    pub(super) fn catalog_type(
        &mut self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, Type>,
    ) -> Type {
        match ty {
            CatalogTypeRef::Core(core) => self.declared_type(DeclaredTypeRef::Core(core)),
            CatalogTypeRef::Standard(standard) => self.standard_type(standard),
            CatalogTypeRef::Parameter(name) => variables[name],
            CatalogTypeRef::FixedArray { element, length } => {
                let element = self.catalog_type(*element, variables);
                Type::Array(self.inference.array_type_with_length(element, Some(length)))
            }
            CatalogTypeRef::Application {
                constructor,
                arguments,
            } => {
                let [value] = arguments else {
                    unreachable!("validated built-in type constructors have one argument")
                };
                let value = self.catalog_type(*value, variables);
                if constructor == StdlibTypeConstructorId::Array {
                    Type::Array(self.array_type_id(value))
                } else if constructor == StdlibTypeConstructorId::Option {
                    Type::Option(self.inference.option_type(value))
                } else if constructor == StdlibTypeConstructorId::Result {
                    Type::Result(self.inference.result_type(value))
                } else if constructor == StdlibTypeConstructorId::Set {
                    Type::Set(self.inference.set_type(value))
                } else {
                    unreachable!("validated catalog type constructor has semantic support")
                }
            }
        }
    }

    pub(super) fn validate_catalog_argument(
        &mut self,
        argument: &Expr,
        rule: ParameterRule,
        item: &StdlibItem,
    ) {
        match rule {
            ParameterRule::Value => {}
            ParameterRule::StringLiteral if !matches!(argument.kind, ExprKind::String(_)) => {
                self.error(
                    format!("`{}` expects a string literal", item.qualified_name),
                    argument.span,
                );
            }
            ParameterRule::SignatureLiteral => match &argument.kind {
                ExprKind::Signature(signature) => {
                    if let Err(message) = parse_signature(signature) {
                        self.error(message, argument.span);
                    }
                }
                _ => self.error(
                    format!("`{}` expects a `sig\"...\"` literal", item.qualified_name),
                    argument.span,
                ),
            },
            ParameterRule::StringLiteral => {}
        }
    }

    pub(super) fn array_type_id(&mut self, element: Type) -> ArrayTypeId {
        self.inference.array_type(element)
    }

    pub(super) fn path(
        &mut self,
        path: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<PathResolution> {
        match path {
            [name, fields @ ..]
                if matches!(
                    name.as_str(),
                    "current" | "old" | "settings" | "oldSettings"
                ) && self.binding(name).is_some() =>
            {
                let binding = self
                    .binding_for_use(name, span)
                    .expect("the shadowing binding was found above");
                let (ty, members) =
                    self.resolve_members_or_defer(binding.ty, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: binding.id.map(ResolvedValue::Variable),
                    members,
                })
            }
            [root] if root == "current" || root == "old" => {
                self.require_state_snapshot(span)?;
                Some(PathResolution {
                    ty: Type::Known(self.inference.type_store().id_for_state_snapshot()),
                    value: Some(if root == "current" {
                        ResolvedValue::CurrentSnapshot
                    } else {
                        ResolvedValue::OldSnapshot
                    }),
                    members: Some(Vec::new()),
                })
            }
            [root, field, fields @ ..] if root == "current" || root == "old" => {
                self.require_state_snapshot(span)?;
                let Some((id, ty)) = self.visible_state_field(field) else {
                    self.unknown_state_field(field, span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "current" {
                    ResolvedValue::CurrentState(id)
                } else {
                    ResolvedValue::OldState(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [root, field, fields @ ..] if root == "settings" || root == "oldSettings" => {
                let Some((id, ty)) = self.declarations.settings.get(field).copied() else {
                    self.error(format!("unknown setting `{field}`"), span);
                    return None;
                };
                let (ty, members) = self.resolve_members_or_defer(ty, fields, span, expression)?;
                let value = if root == "settings" {
                    ResolvedValue::Setting(id)
                } else {
                    ResolvedValue::OldSetting(id)
                };
                Some(PathResolution {
                    ty,
                    value: Some(value),
                    members,
                })
            }
            [root] if root == "settings" || root == "oldSettings" => Some(PathResolution {
                ty: Type::Known(self.inference.type_store().id_for_settings_view()),
                value: Some(if root == "settings" {
                    ResolvedValue::SettingsView
                } else {
                    ResolvedValue::OldSettingsView
                }),
                members: Some(Vec::new()),
            }),
            [name, fields @ ..]
                if self.provider_value.is_some_and(|(provider, _)| {
                    self.standard_library.state_provider(provider).value_name == name
                }) =>
            {
                let (provider, provider_type) = self.provider_value.unwrap();
                let (ty, members) =
                    self.resolve_members_or_defer(provider_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderValue(provider)),
                    members,
                })
            }
            [name, fields @ ..]
                if matches!(self.callable, CallableContext::LibraryFunction(_))
                    && self
                        .standard_library
                        .state_providers()
                        .iter()
                        .any(|provider| provider.value_name == name) =>
            {
                let provider = self
                    .standard_library
                    .state_providers()
                    .iter()
                    .find(|provider| provider.value_name == name)
                    .expect("provider value name was discovered above");
                let provider_type = self.standard_type(provider.process_type);
                let (ty, members) =
                    self.resolve_members_or_defer(provider_type, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: Some(ResolvedValue::ProviderValue(provider.id)),
                    members,
                })
            }
            [name, fields @ ..] => {
                let Some(binding) = self.binding_for_use(name, span) else {
                    let spelling = path.join(".");
                    if let Some(id) = legacy_value_path_diagnostic(&spelling) {
                        let metadata = migration_diagnostic(id)
                            .expect("type checker migration diagnostic IDs must exist");
                        let mut diagnostic = Diagnostic::type_error(metadata.message, span)
                            .with_primary_label(metadata.primary_label);
                        for note in metadata.notes {
                            diagnostic = diagnostic.with_note(*note);
                        }
                        self.errors.push(diagnostic);
                        return None;
                    }
                    let ordinary_rule =
                        foreign_spelling(&spelling, ForeignSpellingContext::ValuePath);
                    let attached_process_rule = foreign_spelling(
                        &spelling,
                        ForeignSpellingContext::AttachedProcessValuePath,
                    );
                    if let Some((rule, requires_attached_process)) = ordinary_rule
                        .map(|rule| (rule, false))
                        .or_else(|| attached_process_rule.map(|rule| (rule, true)))
                    {
                        let mut diagnostic = Diagnostic::type_error(rule.message, span)
                            .with_primary_label(rule.primary_label);
                        let process_context_is_available = self.expression_mode
                            == ExpressionMode::StateSource
                            || matches!(
                                self.callable,
                                CallableContext::LibraryFunction(_)
                                    | CallableContext::Action(
                                        ActionKind::OnAttach
                                            | ActionKind::WhileAttached
                                            | ActionKind::Start
                                            | ActionKind::Split
                                            | ActionKind::Reset
                                            | ActionKind::IsLoading
                                            | ActionKind::GameTime
                                    )
                            );
                        let process_is_available = process_context_is_available
                            && self.provider_value.is_some_and(|(provider, _)| {
                                self.standard_library.state_provider(provider).value_name
                                    == "process"
                            });
                        if !requires_attached_process || process_is_available {
                            diagnostic = diagnostic.with_machine_applicable_fix(
                                rule.fix_title,
                                span,
                                rule.replacement.text(),
                            );
                        } else {
                            diagnostic = diagnostic.with_note(
                                "the `process` value is available only in native-process attachment and attached lifecycle contexts; pass the name into an ordinary function explicitly",
                            );
                        }
                        self.errors.push(diagnostic);
                        return None;
                    }
                    self.error(format!("unknown variable `{name}`"), span);
                    return None;
                };
                let (ty, members) =
                    self.resolve_members_or_defer(binding.ty, fields, span, expression)?;
                Some(PathResolution {
                    ty,
                    value: binding.id.map(ResolvedValue::Variable),
                    members,
                })
            }
            _ => {
                self.error(format!("unknown value `{}`", path.join(".")), span);
                None
            }
        }
    }

    fn require_state_snapshot(&mut self, span: Span) -> Option<()> {
        if self.expression_mode == ExpressionMode::StateSource {
            self.error(
                "a state field cannot read from its own `current` or `old` snapshot",
                span,
            );
            return None;
        }
        if self.callable.is_function() {
            self.error(
                "functions are independent of action snapshots; pass the value as a parameter",
                span,
            );
            return None;
        }
        if !matches!(self.callable, CallableContext::Action(_)) {
            self.error(
                "state snapshots are only available in lifecycle actions",
                span,
            );
            return None;
        }
        if let CallableContext::Action(action @ (ActionKind::Setup | ActionKind::OnAttach)) =
            self.callable
        {
            let message = match action {
                ActionKind::Setup => "state snapshots are not available during `setup`",
                ActionKind::OnAttach => {
                    "state snapshots are not available until `onAttach` completes"
                }
                _ => unreachable!(),
            };
            self.error(message, span);
            return None;
        }
        Some(())
    }

    pub(super) fn resolve_members_or_defer(
        &mut self,
        ty: Type,
        fields: &[String],
        span: Span,
        expression: Option<ExprId>,
    ) -> Option<(Type, Option<Vec<ResolvedMember>>)> {
        if fields.is_empty() {
            return Some((ty, Some(Vec::new())));
        }
        if self.is_error_type(ty) {
            return Some((ty, None));
        }
        if matches!(self.shallow_type(ty), Type::Variable(_))
            && let Some(expression) = expression
        {
            let result = self.fresh_inference(Requirements::none(), None);
            self.deferred_member_paths.push(DeferredMemberPath {
                expression,
                receiver: ty,
                fields: fields.to_vec(),
                result,
                span,
            });
            return Some((result, None));
        }
        let (ty, members) = self.resolve_members(ty, fields, span)?;
        Some((ty, Some(members)))
    }

    pub(super) fn resolve_members(
        &mut self,
        mut ty: Type,
        fields: &[String],
        span: Span,
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for field in fields {
            let shallow_type = self.shallow_type(ty);
            let (next, member) = self.resolve_member(shallow_type, field, span)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    pub(super) fn resolve_member(
        &mut self,
        ty: Type,
        field: &str,
        span: Span,
    ) -> Option<(Type, ResolvedMember)> {
        if let Some(resolved) = self.lookup_member(ty, field) {
            return Some(resolved);
        }
        if self.standard_type_id(ty) == Some(StdlibTypeId::String)
            && let Some(id) = legacy_string_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(id, name_span, None);
            return None;
        }
        if matches!(ty, Type::Array(_))
            && let Some(id) = legacy_array_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(
                id,
                name_span,
                Some((
                    if field == "Length" {
                        "replace `Length` with `length()`"
                    } else {
                        "replace `Count` with `length()`"
                    },
                    "length()",
                )),
            );
            return None;
        }
        if matches!(ty, Type::Set(_))
            && let Some(id) = legacy_set_field_diagnostic(field)
        {
            let name_span = member_name_span(span, field);
            self.migration_member_error(
                id,
                name_span,
                Some(("replace `Count` with `length()`", "length()")),
            );
            return None;
        }
        match ty {
            Type::Known(id)
                if matches!(
                    self.inference.type_store().kind(id),
                    TypeKind::StateSnapshot
                ) =>
            {
                self.unknown_state_field(field, span)
            }
            Type::Known(_) if self.source_record_id(ty).is_some() => {
                self.error(format!("unknown record field `{field}`"), span)
            }
            Type::Known(id) => {
                let name = self.type_name(Type::Known(id));
                self.error(format!("{name} has no field `{field}`"), span)
            }
            _ => {
                let ty = self.type_name(ty);
                self.error(format!("`{field}` cannot be accessed on `{ty}`"), span);
            }
        }
        None
    }

    fn migration_member_error(
        &mut self,
        id: crate::migration::MigrationDiagnosticId,
        span: Span,
        fix: Option<(&str, &str)>,
    ) {
        let metadata =
            migration_diagnostic(id).expect("type checker migration diagnostic IDs must exist");
        let mut diagnostic = Diagnostic::type_error(metadata.message, span)
            .with_primary_label(metadata.primary_label);
        for note in metadata.notes {
            diagnostic = diagnostic.with_note(*note);
        }
        if let Some((title, replacement)) = fix {
            diagnostic = diagnostic.with_machine_applicable_fix(title, span, replacement);
        }
        self.errors.push(diagnostic);
    }

    pub(super) fn lookup_member(&self, ty: Type, field: &str) -> Option<(Type, ResolvedMember)> {
        if matches!(
            ty,
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::StateSnapshot)
        ) {
            return self
                .visible_state_field(field)
                .map(|(field, ty)| (ty, ResolvedMember::StateField(field)));
        }
        if matches!(
            ty,
            Type::Known(id)
                if matches!(self.inference.type_store().kind(id), TypeKind::SettingsView)
        ) {
            return self
                .declarations
                .settings
                .get(field)
                .map(|(setting, ty)| (*ty, ResolvedMember::SettingField(*setting)));
        }
        if let Some(owner) = self.standard_type_id(ty)
            && let Some(field) = self.visible_standard_field(owner, field)
        {
            return Some((
                self.standard_field_type(field.id),
                ResolvedMember::StandardField(field.id),
            ));
        }
        match self.source_record_id(ty) {
            Some(record_id) => self
                .declarations
                .records
                .iter()
                .find(|record| record.id == record_id)
                .and_then(|record| record.fields.iter().find(|item| item.name == field))
                .map(|field| {
                    (
                        self.syntax_type(field.ty),
                        ResolvedMember::RecordField(field.id),
                    )
                }),
            None => None,
        }
    }

    fn visible_state_field(&self, name: &str) -> Option<(crate::ast::ValueId, Type)> {
        self.declarations
            .state_fields
            .get(name)
            .copied()
            .or_else(|| {
                self.active_state_layout.and_then(|layout| {
                    self.declarations
                        .layout_state_fields
                        .get(&layout)
                        .and_then(|fields| fields.get(name))
                        .copied()
                })
            })
    }

    fn unknown_state_field(&mut self, name: &str, span: Span) {
        let layouts = self
            .declarations
            .layout_state_fields
            .values()
            .filter(|fields| fields.contains_key(name))
            .count();
        if layouts != 0 {
            self.error(
                format!(
                    "state field `{name}` is layout-specific; access it inside the corresponding `match layout` arm"
                ),
                span,
            );
        } else {
            self.error(format!("unknown state field `{name}`"), span);
        }
    }

    fn visible_standard_field(
        &self,
        owner: StdlibTypeId,
        name: &str,
    ) -> Option<&'static crate::stdlib::StdlibField> {
        self.standard_library.public_field(owner, name).or_else(|| {
            let CallableContext::LibraryFunction(item) = self.callable else {
                return None;
            };
            (self.standard_library.item(item).owner == StdlibOwner::Type(owner))
                .then(|| {
                    self.standard_library
                        .fields_of(owner)
                        .find(|field| field.name == name)
                })
                .flatten()
        })
    }

    pub(super) fn resolve_deferred_member_paths(&mut self) {
        let mut pending = std::mem::take(&mut self.deferred_member_paths);
        loop {
            let mut unresolved = Vec::new();
            let mut made_progress = false;
            for deferred in pending {
                let receiver = self.shallow_type(deferred.receiver);
                if matches!(receiver, Type::Variable(_)) {
                    unresolved.push(deferred);
                    continue;
                }
                self.finish_deferred_member_path(deferred, receiver);
                made_progress = true;
            }
            pending = unresolved;
            if pending.is_empty() {
                return;
            }

            let mut variables = Vec::new();
            for deferred in &pending {
                if let Type::Variable(variable) = self.shallow_type(deferred.receiver)
                    && !variables.contains(&variable)
                {
                    variables.push(variable);
                }
            }
            for variable in variables {
                let constraints = pending
                    .iter()
                    .filter(|deferred| {
                        self.shallow_type(deferred.receiver) == Type::Variable(variable)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let mut candidates = self.member_receiver_types();
                candidates.retain(|candidate| {
                    constraints.iter().all(|constraint| {
                        let Some((result, _)) = self.lookup_members(*candidate, &constraint.fields)
                        else {
                            return false;
                        };
                        match self.shallow_type(constraint.result) {
                            Type::Variable(_) => true,
                            expected => result == expected,
                        }
                    })
                });
                if let [candidate] = candidates.as_slice() {
                    self.unify(Type::Variable(variable), *candidate, constraints[0].span);
                    made_progress = true;
                }
            }
            if !made_progress {
                break;
            }
        }

        let mut diagnosed = HashSet::new();
        for deferred in &pending {
            let Type::Variable(variable) = self.shallow_type(deferred.receiver) else {
                continue;
            };
            if !diagnosed.insert(variable) {
                continue;
            }
            let constraints = pending
                .iter()
                .filter(|candidate| {
                    self.shallow_type(candidate.receiver) == Type::Variable(variable)
                })
                .collect::<Vec<_>>();
            let mut candidates = self.member_receiver_types();
            candidates.retain(|candidate| {
                constraints.iter().all(|constraint| {
                    self.lookup_members(*candidate, &constraint.fields)
                        .is_some()
                })
            });
            let fields = constraints
                .iter()
                .flat_map(|constraint| constraint.fields.iter())
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = if candidates.is_empty() {
                format!("cannot infer a type that provides the accessed fields {fields}")
            } else {
                let candidates = candidates
                    .into_iter()
                    .map(|candidate| self.type_name(candidate))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "member access does not uniquely determine its receiver type; fields {fields} match {candidates}"
                )
            };
            self.error(message, constraints[0].span);
        }
    }

    pub(super) fn finish_deferred_member_path(
        &mut self,
        deferred: DeferredMemberPath,
        receiver: Type,
    ) {
        let Some((result, members)) =
            self.resolve_members(receiver, &deferred.fields, deferred.span)
        else {
            return;
        };
        if self.unify(result, deferred.result, deferred.span).is_some() {
            self.semantics
                .resolve_path_members(deferred.expression, members);
        }
    }

    pub(super) fn lookup_members(
        &self,
        mut ty: Type,
        fields: &[String],
    ) -> Option<(Type, Vec<ResolvedMember>)> {
        let mut members = Vec::with_capacity(fields.len());
        for field in fields {
            let (next, member) = self.lookup_member(ty, field)?;
            ty = next;
            members.push(member);
        }
        Some((ty, members))
    }

    pub(super) fn member_receiver_types(&self) -> Vec<Type> {
        self.standard_library
            .types()
            .iter()
            .filter(|ty| self.standard_library.public_fields(ty.id).next().is_some())
            .map(|ty| self.standard_type(ty.id))
            .chain([Type::Known(
                self.inference.type_store().id_for_state_snapshot(),
            )])
            .chain([Type::Known(
                self.inference.type_store().id_for_settings_view(),
            )])
            .chain(
                self.declarations
                    .records
                    .iter()
                    .map(|record| self.record_type(record.id)),
            )
            .collect()
    }

    pub(super) fn type_name(&mut self, ty: Type) -> String {
        let ty = self.shallow_type(ty);
        match ty {
            Type::Array(array) => {
                let element = self.inference.array_element(array);
                let element = self.type_name(element);
                match self.inference.array_length(array) {
                    Some(length) => format!("[{element}; {length}]"),
                    None => format!("[{element}]"),
                }
            }
            Type::Option(option) => {
                let value = self.inference.option_value(option);
                format!("{}?", self.type_name(value))
            }
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                format!("{}!", self.type_name(value))
            }
            Type::Async(future) => {
                let value = self.inference.async_value(future);
                format!("async {}", self.type_name(value))
            }
            Type::Set(set) => {
                let element = self.inference.set_element(set);
                format!("Set<{}>", self.type_name(element))
            }
            Type::Variable(_) => "an inferred type".to_owned(),
            Type::Known(id) => match self.inference.type_store().kind(id) {
                TypeKind::Record(record) => self
                    .declarations
                    .records
                    .iter()
                    .find(|candidate| candidate.id == *record)
                    .map_or_else(|| ty.to_string(), |record| record.name.clone()),
                TypeKind::Enum(enumeration) => self
                    .declarations
                    .enums
                    .iter()
                    .find(|candidate| candidate.id == *enumeration)
                    .map_or_else(|| ty.to_string(), |enumeration| enumeration.name.clone()),
                TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
                TypeKind::SettingsView => "SettingsView".to_owned(),
                _ => self.inference.known_type_name(id),
            },
        }
    }

    pub(super) fn inference_error_message(&mut self, error: InferenceError) -> String {
        match error {
            InferenceError::Message(message) => message,
            InferenceError::TypeMismatch { left, right } => format!(
                "types do not match: `{}` and `{}`",
                self.type_name(left),
                self.type_name(right)
            ),
            InferenceError::UnsupportedOperation(ty) => {
                format!(
                    "type `{}` does not support this operation",
                    self.type_name(ty)
                )
            }
            InferenceError::UnsatisfiedConstraints(ty) => format!(
                "type `{}` does not satisfy the inferred constraints",
                self.type_name(ty)
            ),
            InferenceError::IntegerLiteralOutOfRange(ty) => {
                format!("integer literal does not fit in `{}`", self.type_name(ty))
            }
        }
    }
}

fn member_name_span(span: Span, name: &str) -> Span {
    Span {
        start: span.end.saturating_sub(name.len()),
        end: span.end,
    }
}
