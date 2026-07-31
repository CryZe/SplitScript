//! Call, path, catalog-overload, and member resolution during type checking.

use std::collections::{HashMap, HashSet};

use crate::{
    Diagnostic,
    ast::{ActionKind, ArrayTypeId, Expr, ExprId, ExprKind, Span},
    inference::{InferenceError, Requirements, Type, type_may_have_capability},
    semantic::{PendingResolvedCall, ResolvedMember, ResolvedValue},
    signature::parse_signature,
    stdlib::{
        Availability, DeclaredTypeRef, ItemKind, ParameterRule, StdlibItem, StdlibItemId,
        StdlibTypeConstructorId, StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    stdlib_semantic::{CallCandidate, StandardLibrarySemanticExt},
    types::TypeKind,
};

use super::{
    Checker, DeferredMemberPath, MethodReceiver, PathResolution, catalog_type_argument,
    closest_name,
    context::{CallableContext, ExpressionMode},
};

impl Checker {
    pub(super) fn call(
        &mut self,
        callee: &[String],
        name_span: Span,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        if callee == ["Some"] {
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

        if callee == ["Ok"] {
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

        if callee == ["Err"] {
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

        if self.provider_value.is_some() && callee.first().is_some_and(|root| root == "process") {
            let provider = self
                .standard_library
                .state_provider(self.provider_value.unwrap().0);
            self.error(
                format!(
                    "`process` is unavailable under `state {}`; use `{}` instead",
                    provider.name, provider.value_name
                ),
                span,
            );
            return None;
        }
        let standard_library = self.standard_library.clone();
        let mut function_candidates = standard_library.function_candidates(callee);
        if function_candidates.len() > 1 {
            self.ambiguous_catalog_call(callee, &function_candidates, span);
            return None;
        }
        if let Some(candidate) = function_candidates.pop() {
            return self.catalog_call(&candidate, None, args, expected, expression, span);
        }
        let (display_name, signature, parameters, resolved_call) = if let [name] = callee {
            let Some(signature) = self.declarations.functions.get(name).cloned() else {
                let suggestion = self.function_name_suggestion(callee);
                self.unknown_function(callee, name_span, span, suggestion.as_deref());
                return None;
            };
            (
                name.clone(),
                signature.clone(),
                signature.params,
                PendingResolvedCall::UserFunction {
                    function: signature.id,
                },
            )
        } else {
            if let Some(suggestion) = self.function_name_suggestion(callee) {
                self.unknown_function(callee, name_span, span, Some(&suggestion));
                return None;
            }
            let (method, receiver_path) = callee.split_last().unwrap();
            let receiver = self.path(receiver_path, span, None)?;
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
            if candidates.len() > 1 {
                self.ambiguous_catalog_call(callee, &candidates, span);
                return None;
            }
            if let Some(candidate) = candidates.pop() {
                return self.catalog_call(
                    &candidate,
                    Some(MethodReceiver {
                        ty: receiver_type,
                        value: receiver_value,
                        members: receiver_members,
                    }),
                    args,
                    expected,
                    expression,
                    span,
                );
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
            let receiver_name = self.type_name(receiver_type);
            (
                format!("{receiver_name}.{method}"),
                signature.clone(),
                signature.params.into_iter().skip(1).collect(),
                PendingResolvedCall::UserMethod {
                    function: signature.id,
                    receiver: receiver_value,
                    receiver_type,
                    receiver_members,
                },
            )
        };
        if self.declarations.debug_functions.contains(&signature.id)
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
        let result = self.expect_expression(expression, signature.result, expected, span)?;
        self.semantics.resolve_call(expression, resolved_call);
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
            let candidate = CallCandidate {
                item,
                type_arguments: Vec::new(),
            };
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

    pub(super) fn catalog_call(
        &mut self,
        candidate: &CallCandidate,
        receiver: Option<MethodReceiver>,
        args: &[Expr],
        expected: Option<Type>,
        expression: ExprId,
        span: Span,
    ) -> Option<Type> {
        let item = candidate.item;
        let mut variables = HashMap::new();
        for parameter in item.signature.type_parameters {
            let requirements = parameter
                .constraints
                .iter()
                .fold(Requirements::none(), |requirements, constraint| {
                    requirements | Requirements::capability(*constraint)
                });
            let ty = candidate
                .type_arguments
                .iter()
                .find(|(name, _)| *name == parameter.name)
                .map(|(_, ty)| self.inference.known_builtin(*ty))
                .unwrap_or_else(|| self.fresh_inference(requirements.clone(), None));
            if !requirements.is_empty() {
                self.require(ty, requirements, span)?;
            }
            variables.insert(parameter.name, ty);
        }
        if item.id == StdlibItemId::ProcessRead && candidate.type_arguments.is_empty() {
            self.inferred_process_reads.push((variables["T"], span));
        }
        if let Some(receiver) = &receiver {
            let declared_receiver = self.catalog_type(
                candidate
                    .receiver()
                    .expect("method candidates declare a receiver"),
                &variables,
            );
            self.unify(receiver.ty, declared_receiver, span)?;
        }
        let expected_result = expected.map(|ty| self.shallow_type(ty));
        let result_type = if let (Some(value), Some(Type::Result(result))) = (
            catalog_type_argument(item.signature.result, StdlibTypeConstructorId::Result),
            expected_result,
        ) {
            let declared_value = self.catalog_type(value, &variables);
            let expected_value = self.inference.result_value(result);
            self.unify(declared_value, expected_value, span)?;
            Type::Result(result)
        } else {
            self.catalog_type(item.signature.result, &variables)
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
        }
        let operation = item.operation_semantics();
        if operation.availability == Availability::OnAttach
            && !(self.expression_mode == ExpressionMode::SuspensionOperand
                && matches!(self.callable, CallableContext::Action(ActionKind::OnAttach)))
        {
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
                receiver: receiver.as_ref().map(|receiver| receiver.value),
                receiver_type: receiver.as_ref().map(|receiver| receiver.ty),
                receiver_members: receiver
                    .map(|receiver| receiver.members)
                    .unwrap_or_default(),
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
            [root, field, fields @ ..] if root == "current" || root == "old" => {
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
                if matches!(self.callable, CallableContext::Action(ActionKind::OnAttach)) {
                    self.error(
                        "state snapshots are not available until `onAttach` completes",
                        span,
                    );
                    return None;
                }
                let Some((id, ty)) = self.declarations.state_fields.get(field).copied() else {
                    self.error(format!("unknown state field `{field}`"), span);
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
            [name, fields @ ..] => {
                let Some(binding) = self.binding_for_use(name, span) else {
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
        match ty {
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

    pub(super) fn lookup_member(&self, ty: Type, field: &str) -> Option<(Type, ResolvedMember)> {
        if let Some(owner) = self.standard_type_id(ty)
            && let Some(field) = self.standard_library.public_field(owner, field)
        {
            return Some((
                self.declared_type(field.ty),
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
                format!("Array<{}>", self.type_name(element))
            }
            Type::Option(option) => {
                let value = self.inference.option_value(option);
                format!("{}?", self.type_name(value))
            }
            Type::Result(result) => {
                let value = self.inference.result_value(result);
                format!("{}!", self.type_name(value))
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
