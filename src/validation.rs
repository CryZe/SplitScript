//! Post-type-check semantic validation.
//!
//! Type inference establishes resolved types and calls. This stage derives
//! operational/capability facts and reports constraints that require the
//! complete typed program. Strict compilation and editor recovery consume the
//! same product.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    Diagnostic, DiagnosticCode, DiagnosticFix, FixApplicability, TextEdit,
    ast::{self, EnumDecl, Program, StateSource},
    capabilities::CapabilityAnalysis,
    effects::{OperationAnalysis, StateSnapshotContext},
    hir::{
        self, ExpressionResolution, TypedBlock, TypedExpression, TypedExpressionKind,
        TypedMatchArm, TypedProgram, TypedStatementKind, TypedVisitor,
    },
    semantic::{
        DynamicCallCallee, FunctionInstance, ResolvedCall, ResolvedEnumVariantId, ResolvedMember,
        ResolvedValue, SemanticModel,
    },
    stdlib::{
        Implementation, StandardLibrary, StdlibCapabilityId, StdlibItemId, StdlibOwner,
        StdlibTypeConstructorId,
    },
    types::TypeKind,
    visit::{self, Visitor as SyntaxVisitor},
};

mod stdlib_bodies;

pub(crate) struct ValidationOutput {
    pub(crate) attachment_globals: crate::attachment_globals::AttachmentGlobalAnalysis,
    pub(crate) capabilities: CapabilityAnalysis,
    pub(crate) effects: OperationAnalysis,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

pub(crate) fn validate(
    standard_library: StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    enum_types: &[EnumDecl],
) -> ValidationOutput {
    let (attachment_globals, attachment_diagnostics) =
        crate::attachment_globals::analyze(syntax, hir, semantics);
    let capabilities = CapabilityAnalysis::build(
        &syntax.records,
        enum_types,
        &syntax.functions,
        semantics,
        standard_library.clone(),
    );
    let effects = OperationAnalysis::infer(hir, semantics, &capabilities);
    let mut diagnostics = Vec::new();
    diagnostics.extend(attachment_diagnostics);
    diagnostics.extend(stdlib_bodies::validate_signatures(
        &standard_library,
        syntax,
        hir,
        semantics,
    ));
    diagnostics.extend(validate_function_instances(syntax, hir, semantics));
    diagnostics.extend(validate_future_storage(syntax, semantics, enum_types));
    diagnostics.extend(validate_must_use(&standard_library, hir, semantics));
    diagnostics.extend(validate_unused_bindings(syntax, hir));
    diagnostics.extend(validate_unused_declarations(
        syntax,
        hir,
        semantics,
        &capabilities,
    ));
    diagnostics.extend(validate_static_setting_lookups(syntax, hir));
    diagnostics.extend(validate_async_function_results(
        &standard_library,
        syntax,
        hir,
        &effects,
    ));
    diagnostics.extend(validate_async_recursion(syntax, hir, &effects));
    diagnostics.extend(validate_suspending_calls(
        &standard_library,
        syntax,
        hir,
        &effects,
    ));

    // The standalone standard-library bootstrap is the authority for catalog
    // metadata. Every ordinary compilation rechecks the same injected bodies
    // and verifies that their complete typed call graph still agrees with the
    // cached, user-independent result.
    if standard_library.source_body_operations_are_initialized() {
        for item in standard_library.all_items() {
            if !matches!(
                item.implementation,
                Implementation::LibraryBody { .. } | Implementation::LibraryOverloads { .. }
            ) {
                continue;
            }
            let cataloged = standard_library.operation_metadata(item.id);
            let mut functions = hir.library_functions(item.id);
            // Signature-only tooling contexts deliberately omit catalog body
            // declarations. Their operation metadata was already derived by
            // the standalone bootstrap, so there is no local body to compare.
            let Some(function) = functions.next() else {
                continue;
            };
            let inferred = functions.fold(
                effects.function(function).metadata(),
                |combined, function| {
                    combined.conservative_union(effects.function(function).metadata())
                },
            );
            if inferred != cataloged {
                let span = syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.span)
                    .unwrap_or_default();
                diagnostics.push(Diagnostic::semantic(
                    format!(
                        "standard-library body `{}` inferred operation metadata {:?}, but its standalone catalog analysis produced {:?}",
                        item.qualified_name, inferred, cataloged
                    ),
                    span,
                ));
            }
        }
    }

    for violation in effects.attached_process_violations(hir, semantics, &capabilities) {
        let name = violation
            .standard_library_name
            .map(str::to_owned)
            .or_else(|| {
                let function = violation.function?;
                syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.name.clone())
            });
        diagnostics.push(Diagnostic::semantic(
            format!(
                "`{}` requires an attached process and is unavailable in `{}`",
                name.unwrap_or_else(|| "function".to_owned()),
                violation.action.name(),
            ),
            violation.expression_span,
        ));
    }

    for violation in effects.state_snapshot_violations(hir, semantics, &capabilities) {
        let name = violation
            .standard_library_name
            .map(str::to_owned)
            .or_else(|| {
                let function = violation.function?;
                syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == function)
                    .map(|declaration| declaration.name.clone())
            })
            .unwrap_or_else(|| "function".to_owned());
        let context = match violation.context {
            StateSnapshotContext::Action(action) => format!("`{}`", action.name()),
            StateSnapshotContext::StateSource => "a state field expression".to_owned(),
            StateSnapshotContext::StateTransform => "a state field filter".to_owned(),
        };
        diagnostics.push(
            Diagnostic::semantic(
                format!("`{name}` requires state snapshots and is unavailable in {context}"),
                violation.expression_span,
            )
            .with_migration_topic("asl.state.helper-snapshots"),
        );
    }

    for expression in hir.expressions() {
        if let TypedExpressionKind::Binary {
            op: ast::BinaryOp::Eq | ast::BinaryOp::Ne,
            left,
            ..
        } = expression.kind
        {
            let operand = hir
                .expression(left)
                .expect("binary operands belong to typed HIR");
            if let Err(error) =
                capabilities.require(operand.ty, StdlibCapabilityId::Equatable, semantics)
            {
                diagnostics.push(Diagnostic::semantic(error, expression.span));
            }
        }
        if let Some(ResolvedCall::StandardLibrary {
            item,
            type_arguments,
            ..
        }) = hir.call(expression.id)
        {
            let item = standard_library.item(*item);
            for (parameter, argument) in item.signature.type_parameters.iter().zip(type_arguments) {
                for constraint in parameter.constraints {
                    if let Err(error) = capabilities.require(*argument, *constraint, semantics) {
                        let capability = standard_library.capability(*constraint);
                        diagnostics.push(capability_diagnostic(
                            format!(
                                "`{}` requires capability `{}`: {error}",
                                item.qualified_name, capability.name,
                            ),
                            expression.span,
                            *argument,
                            *constraint,
                            syntax,
                            semantics,
                            &standard_library,
                            &capabilities,
                        ));
                    }
                }
            }
        }
        let display_source = match &expression.kind {
            TypedExpressionKind::InterpolatedString(parts) => {
                for part in parts {
                    let hir::TypedInterpolatedPart::Expression {
                        expression: value,
                        conversion: Some(hir::ImplicitConversion::ToString { source }),
                    } = part
                    else {
                        continue;
                    };
                    if let Err(error) =
                        capabilities.require(*source, StdlibCapabilityId::Display, semantics)
                    {
                        let span = hir
                            .expression(*value)
                            .expect("interpolation operands belong to typed HIR")
                            .span;
                        diagnostics.push(capability_diagnostic(
                            error,
                            span,
                            *source,
                            StdlibCapabilityId::Display,
                            syntax,
                            semantics,
                            &standard_library,
                            &capabilities,
                        ));
                    }
                }
                None
            }
            TypedExpressionKind::Cast {
                expression: value, ..
            } if matches!(
                semantics.types().kind(expression.ty),
                TypeKind::Standard(crate::stdlib::StdlibTypeId::String)
            ) =>
            {
                hir.expression(*value).map(|value| (value.ty, value.span))
            }
            _ => None,
        };
        if let Some((source, span)) = display_source
            && let Err(error) = capabilities.require(source, StdlibCapabilityId::Display, semantics)
        {
            diagnostics.push(capability_diagnostic(
                error,
                span,
                source,
                StdlibCapabilityId::Display,
                syntax,
                semantics,
                &standard_library,
                &capabilities,
            ));
        }
    }

    if let Some(state) = &syntax.state {
        for field in state.all_fields() {
            if matches!(
                field.source,
                StateSource::Pointer(ref path) if path.decoder.is_none()
            ) {
                let ty = semantics
                    .value_type(field.id)
                    .expect("checked state fields have semantic types");
                let memory_ty = match semantics.types().kind(ty) {
                    crate::types::TypeKind::Option { value, .. } => *value,
                    _ => ty,
                };
                if let Err(error) =
                    capabilities.require(memory_ty, StdlibCapabilityId::MemoryReadable, semantics)
                {
                    diagnostics.push(Diagnostic::semantic(error, field.span));
                }
            }
        }
    }

    ValidationOutput {
        attachment_globals,
        capabilities,
        effects,
        diagnostics,
    }
}

fn capability_diagnostic(
    message: String,
    span: ast::Span,
    ty: crate::types::TypeId,
    capability: StdlibCapabilityId,
    syntax: &Program,
    semantics: &SemanticModel,
    standard_library: &StandardLibrary,
    capabilities: &CapabilityAnalysis,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::semantic(message, span);
    if standard_library.capability(capability).behavior
        != crate::stdlib::CapabilityBehavior::StructuralMethods
    {
        return diagnostic;
    }

    for &requirement in capabilities.structural_method_requirements(capability) {
        if let Some(function) = capabilities.method_candidate(ty, requirement)
            && let Some(declaration) = syntax
                .functions
                .iter()
                .find(|declaration| declaration.id == function)
        {
            return diagnostic.with_secondary_label(
                declaration.name_span,
                format!(
                    "this method was considered for `{}`",
                    standard_library.render_signature(requirement)
                ),
            );
        }
    }

    let declaration = match semantics.types().kind(ty) {
        TypeKind::Record(id) => syntax
            .records
            .iter()
            .find(|declaration| declaration.id == *id)
            .map(|declaration| declaration.name_span),
        TypeKind::Enum(id) => syntax
            .enum_declarations()
            .find(|declaration| declaration.id == *id)
            .map(|declaration| declaration.name_span),
        _ => None,
    };
    if let Some(declaration) = declaration {
        diagnostic = diagnostic.with_secondary_label(
            declaration,
            format!(
                "define the required method on this type to satisfy `{}`",
                standard_library.capability(capability).name
            ),
        );
    }
    diagnostic
}

fn validate_static_setting_lookups(syntax: &Program, hir: &TypedProgram) -> Vec<Diagnostic> {
    let settings = syntax
        .settings
        .iter()
        .map(|setting| (setting.runtime_key(), setting))
        .collect::<HashMap<_, _>>();
    let mut diagnostics = Vec::new();

    for expression in hir.expressions() {
        let Some(ResolvedCall::StandardLibrary { item, receiver, .. }) = hir.call(expression.id)
        else {
            continue;
        };
        if !matches!(
            *item,
            StdlibItemId::SettingsViewEnabled | StdlibItemId::SettingsViewContains
        ) {
            continue;
        }
        let TypedExpressionKind::Call { arguments, .. } = &expression.kind else {
            continue;
        };
        let Some(argument) = arguments
            .first()
            .and_then(|argument| hir.expression(*argument))
        else {
            continue;
        };
        let TypedExpressionKind::String(key) = &argument.kind else {
            continue;
        };
        let Some(setting) = settings.get(key.as_str()) else {
            continue;
        };
        if matches!(setting.kind, ast::SettingKind::Title { .. }) {
            continue;
        }

        let direct_root = receiver.as_ref().and_then(|receiver| match receiver {
            crate::semantic::ResolvedReceiver::Path { root, members } if members.is_empty() => {
                match root {
                    ResolvedValue::SettingsView => Some("settings"),
                    ResolvedValue::OldSettingsView => Some("oldSettings"),
                    _ => None,
                }
            }
            _ => None,
        });

        if *item == StdlibItemId::SettingsViewEnabled {
            if !matches!(setting.kind, ast::SettingKind::Bool { .. }) || !setting.source_visible {
                continue;
            }
            let Some(root) = direct_root else {
                continue;
            };
            let replacement = format!("{root}.{}", setting.name);
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::StaticSettingLookup,
                    format!("literal setting key `{key}` has a typed member"),
                    expression.span,
                )
                .with_primary_label(format!("use `{replacement}` for this declared setting"))
                .with_secondary_label(setting.name_span, "the setting is declared here")
                .with_machine_applicable_fix(
                    format!("replace the string lookup with `{replacement}`"),
                    expression.span,
                    replacement,
                ),
            );
            continue;
        }

        let mut diagnostic = Diagnostic::warning(
            DiagnosticCode::StaticSettingLookup,
            format!("`contains({})` is always true", quote_string_literal(key)),
            argument.span,
        )
        .with_primary_label("this exact value-setting key is declared statically")
        .with_secondary_label(setting.name_span, "the setting is declared here")
        .with_machine_applicable_fix(
            "replace the known membership test with `true`",
            expression.span,
            "true",
        )
        .with_note(
            "use `contains` with a computed key when declaration membership is genuinely data-driven",
        );

        if setting.source_visible
            && let Some(root) = direct_root
        {
            let member = format!("{root}.{}", setting.name);
            diagnostic = diagnostic.with_note(format!(
                "read the declared setting's typed value as `{member}`"
            ));
            if matches!(setting.kind, ast::SettingKind::Bool { .. }) {
                diagnostic = diagnostic.with_fix(DiagnosticFix {
                    title: format!("read the enabled value with `{member}`"),
                    applicability: FixApplicability::MaybeIncorrect,
                    edits: vec![TextEdit {
                        span: expression.span,
                        replacement: member,
                    }],
                });
            }
        }
        diagnostics.push(diagnostic);
    }

    diagnostics
}

fn validate_async_recursion(
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct Calls {
        values: HashSet<ast::FunctionId>,
    }
    impl TypedVisitor for Calls {
        fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
            if let Some(call) = program.call(expression.id) {
                match call {
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => {
                        self.values.insert(*function);
                    }
                    ResolvedCall::StandardLibrary { item, .. } => {
                        for function in program.library_functions(*item) {
                            self.values.insert(function);
                        }
                    }
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => {}
                }
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    let mut graph = HashMap::<ast::FunctionId, HashSet<ast::FunctionId>>::new();
    for body in hir.all_function_bodies() {
        let mut calls = Calls::default();
        calls.visit_block(&body.body, hir);
        graph
            .entry(body.function.function)
            .or_default()
            .extend(calls.values);
    }

    fn reaches(
        target: ast::FunctionId,
        current: ast::FunctionId,
        graph: &HashMap<ast::FunctionId, HashSet<ast::FunctionId>>,
        visited: &mut HashSet<ast::FunctionId>,
    ) -> bool {
        graph.get(&current).is_some_and(|callees| {
            callees.contains(&target)
                || callees
                    .iter()
                    .copied()
                    .any(|callee| visited.insert(callee) && reaches(target, callee, graph, visited))
        })
    }

    syntax
        .functions
        .iter()
        .filter(|function| {
            effects.function(function.id).suspension == crate::stdlib::SuspensionKind::Suspends
                && reaches(function.id, function.id, &graph, &mut HashSet::new())
        })
        .map(|function| {
            Diagnostic::semantic(
                format!("async function `{}` cannot be recursive yet", function.name),
                function.name_span,
            )
            .with_primary_label("recursive future-frame allocation has no configured limit")
            .with_note(
                "rewrite the recursion as a loop until bounded recursive futures are specified",
            )
        })
        .collect()
}

fn validate_future_storage(
    syntax: &Program,
    semantics: &SemanticModel,
    enum_types: &[EnumDecl],
) -> Vec<Diagnostic> {
    fn contains_future(
        ty: crate::types::TypeId,
        syntax: &Program,
        semantics: &SemanticModel,
        enum_types: &[EnumDecl],
        visited: &mut HashSet<crate::types::TypeId>,
    ) -> bool {
        if !visited.insert(ty) {
            return false;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => false,
            TypeKind::Async { .. } => true,
            TypeKind::Record(record) => syntax
                .records
                .iter()
                .find(|declaration| declaration.id == *record)
                .is_some_and(|declaration| {
                    declaration.fields.iter().any(|field| {
                        semantics.record_field_type(field.id).is_some_and(|field| {
                            contains_future(field, syntax, semantics, enum_types, visited)
                        })
                    })
                }),
            TypeKind::Enum(enumeration) => enum_types
                .iter()
                .find(|declaration| declaration.id == *enumeration)
                .is_some_and(|declaration| {
                    declaration.variants.iter().any(|variant| {
                        semantics
                            .enum_variant_payload(variant.id)
                            .is_some_and(|payload| {
                                contains_future(payload, syntax, semantics, enum_types, visited)
                            })
                    })
                }),
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Range { bound: element, .. } => {
                contains_future(*element, syntax, semantics, enum_types, visited)
            }
            TypeKind::Application { arguments, .. } => arguments
                .iter()
                .any(|argument| contains_future(*argument, syntax, semantics, enum_types, visited)),
            TypeKind::Callable { .. } => false,
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::GenericParameter { .. } => false,
        }
    }

    syntax
        .globals
        .iter()
        .filter(|global| {
            semantics.value_type(global.id).is_some_and(|ty| {
                contains_future(ty, syntax, semantics, enum_types, &mut HashSet::new())
            })
        })
        .map(|global| {
            Diagnostic::semantic(
                format!(
                    "global `{}` cannot store a process-lifetime async value",
                    global.name
                ),
                global.name_span,
            )
            .with_primary_label("this value may retain a cancelled process operation")
            .with_note(
                "store the future in an onAttach local and await it before the process closes",
            )
        })
        .collect()
}

fn validate_async_function_results(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    let library_functions = standard_library
        .all_items()
        .iter()
        .flat_map(|item| hir.library_functions(item.id))
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();
    for function in &syntax.functions {
        if library_functions.contains(&function.id) {
            continue;
        }
        let inferred_async =
            effects.function(function.id).suspension == crate::stdlib::SuspensionKind::Suspends;
        match (
            function.return_annotation,
            function.return_is_async,
            inferred_async,
        ) {
            (Some(_), false, true) => {
                let annotation = function
                    .return_annotation_span
                    .expect("explicit return annotations retain their span");
                let insertion = ast::Span {
                    start: annotation.start,
                    end: annotation.start,
                };
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "function `{}` suspends, so its explicit result must be marked `async`",
                            function.name
                        ),
                        annotation,
                    )
                    .with_machine_applicable_fix(
                        "mark the result as async",
                        insertion,
                        "async ",
                    ),
                );
            }
            (Some(_), true, false) => {
                let keyword = function
                    .return_async_span
                    .expect("async return annotations retain the keyword span");
                let annotation = function
                    .return_annotation_span
                    .expect("explicit return annotations retain their span");
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "function `{}` is declared async but never suspends",
                            function.name
                        ),
                        keyword,
                    )
                    .with_machine_applicable_fix(
                        "remove `async`",
                        ast::Span {
                            start: keyword.start,
                            end: annotation.start,
                        },
                        "",
                    ),
                );
            }
            _ => {}
        }
    }
    diagnostics
}

fn validate_suspending_calls(
    standard_library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    effects: &OperationAnalysis,
) -> Vec<Diagnostic> {
    #[derive(Default)]
    struct AwaitCollector {
        operands: HashSet<crate::ast::ExprId>,
    }

    impl TypedVisitor for AwaitCollector {
        fn visit_expression(&mut self, expression: &hir::TypedExpression, program: &TypedProgram) {
            if let hir::TypedExpressionKind::Suspend {
                mode: ast::SuspensionMode::Await,
                value,
                ..
            } = expression.kind
            {
                self.operands.insert(value);
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    fn call_semantics(
        call: &ResolvedCall,
        standard_library: &StandardLibrary,
        syntax: &Program,
        hir: &TypedProgram,
        effects: &OperationAnalysis,
    ) -> Option<(crate::stdlib::SuspensionKind, String)> {
        match call {
            ResolvedCall::StandardLibrary { item, .. } => {
                let declaration = standard_library.item(*item);
                let suspension = hir
                    .library_functions(*item)
                    .map(|function| effects.function(function).suspension)
                    .max_by_key(|suspension| match suspension {
                        crate::stdlib::SuspensionKind::None => 0,
                        crate::stdlib::SuspensionKind::Suspends => 1,
                    })
                    .unwrap_or_else(|| standard_library.operation_semantics(*item).suspension);
                Some((suspension, declaration.qualified_name.to_owned()))
            }
            ResolvedCall::UserFunction { function, .. }
            | ResolvedCall::UserMethod { function, .. } => {
                let name = syntax
                    .functions
                    .iter()
                    .find(|declaration| declaration.id == *function)
                    .map(|declaration| declaration.name.clone())
                    .unwrap_or_else(|| "function".to_owned());
                Some((effects.function(*function).suspension, name))
            }
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. } => None,
        }
    }

    fn has_runtime_future_storage(
        call: &ResolvedCall,
        _standard_library: &StandardLibrary,
    ) -> bool {
        match call {
            ResolvedCall::UserFunction { .. } | ResolvedCall::UserMethod { .. } => true,
            ResolvedCall::StandardLibrary { .. } => true,
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. } => false,
        }
    }

    let mut awaited = AwaitCollector::default();
    for function in hir.all_function_bodies() {
        awaited.visit_block(&function.body, hir);
    }
    for action in hir.action_bodies() {
        awaited.visit_block(&action.body, hir);
    }

    let mut diagnostics = Vec::new();
    for expression in hir.expressions() {
        let Some(call) = hir.call(expression.id) else {
            continue;
        };
        let Some((suspension, name)) = call_semantics(call, standard_library, syntax, hir, effects)
        else {
            continue;
        };
        if suspension == crate::stdlib::SuspensionKind::Suspends
            && !awaited.operands.contains(&expression.id)
            && !has_runtime_future_storage(call, standard_library)
        {
            diagnostics.push(Diagnostic::semantic(
                format!("`{name}` suspends and must be awaited"),
                expression.span,
            ));
        }
    }
    for operand in awaited.operands {
        let Some(expression) = hir.expression(operand) else {
            continue;
        };
        let Some(call) = hir.call(operand) else {
            continue;
        };
        let Some((suspension, name)) = call_semantics(call, standard_library, syntax, hir, effects)
        else {
            continue;
        };
        if !suspension.is_awaitable() {
            diagnostics.push(Diagnostic::semantic(
                format!("`{name}` is synchronous and cannot be awaited"),
                expression.span,
            ));
        }
    }
    diagnostics
}

#[derive(Debug, Clone, Copy)]
enum LocalBindingKind {
    Parameter,
    Variable,
    Loop,
    Suspension,
    Pattern,
}

impl LocalBindingKind {
    fn description(self) -> &'static str {
        match self {
            Self::Parameter => "parameter",
            Self::Variable => "variable",
            Self::Loop => "loop binding",
            Self::Suspension => "suspension binding",
            Self::Pattern => "pattern binding",
        }
    }
}

#[derive(Debug)]
struct LocalBinding {
    id: ast::ValueId,
    name: String,
    name_span: ast::Span,
    kind: LocalBindingKind,
}

#[derive(Default)]
struct LocalBindingCollector {
    bindings: Vec<LocalBinding>,
}

impl LocalBindingCollector {
    fn push(&mut self, id: ast::ValueId, name: &str, name_span: ast::Span, kind: LocalBindingKind) {
        self.bindings.push(LocalBinding {
            id,
            name: name.to_owned(),
            name_span,
            kind,
        });
    }
}

impl<'ast> SyntaxVisitor<'ast> for LocalBindingCollector {
    fn visit_variable(&mut self, variable: &'ast ast::VariableDecl) {
        self.push(
            variable.id,
            &variable.name,
            variable.name_span,
            LocalBindingKind::Variable,
        );
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast ast::SuspensionBinding) {
        self.push(
            binding.id,
            &binding.name,
            binding.name_span,
            LocalBindingKind::Suspension,
        );
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast ast::ForBinding) {
        self.push(
            binding.id,
            &binding.name,
            binding.span,
            LocalBindingKind::Loop,
        );
    }

    fn visit_pattern(&mut self, pattern: &'ast ast::MatchPattern) {
        let binding = match pattern {
            ast::MatchPattern::Enum {
                binding: Some(binding),
                ..
            }
            | ast::MatchPattern::OptionSome(Some(binding))
            | ast::MatchPattern::IteratorItem(Some(binding))
            | ast::MatchPattern::ResultSuccess(Some(binding))
            | ast::MatchPattern::ResultError(Some(binding)) => Some(binding),
            ast::MatchPattern::Enum { binding: None, .. }
            | ast::MatchPattern::Bool(_)
            | ast::MatchPattern::Char(_)
            | ast::MatchPattern::String(_)
            | ast::MatchPattern::Int { .. }
            | ast::MatchPattern::FileVersion(_)
            | ast::MatchPattern::None
            | ast::MatchPattern::IteratorEnd
            | ast::MatchPattern::OptionSome(None)
            | ast::MatchPattern::IteratorItem(None)
            | ast::MatchPattern::ResultSuccess(None)
            | ast::MatchPattern::ResultError(None)
            | ast::MatchPattern::Wildcard => None,
        };
        if let Some(binding) = binding {
            self.push(
                binding.id,
                &binding.name,
                binding.name_span,
                LocalBindingKind::Pattern,
            );
        }
        visit::walk_pattern(self, pattern);
    }
}

fn validate_unused_bindings(syntax: &Program, hir: &TypedProgram) -> Vec<Diagnostic> {
    let visible_functions = hir
        .function_bodies()
        .map(|body| body.function.function)
        .collect::<HashSet<_>>();
    let mut collector = LocalBindingCollector::default();
    for function in syntax
        .functions
        .iter()
        .filter(|function| visible_functions.contains(&function.id))
    {
        for (index, parameter) in function.params.iter().enumerate() {
            // Methods receive an implicit `self` parameter whose source span is
            // the receiver type. It is not a user-written binding declaration.
            if function.method_of.is_some() && index == 0 && parameter.name == "self" {
                continue;
            }
            collector.push(
                parameter.id,
                &parameter.name,
                parameter.name_span,
                LocalBindingKind::Parameter,
            );
        }
        collector.visit_block(&function.body);
    }
    for action in &syntax.actions {
        collector.visit_block(&action.body);
    }
    let occupied_names = collector
        .bindings
        .iter()
        .map(|binding| binding.name.clone())
        .chain(syntax.globals.iter().map(|global| global.name.clone()))
        .collect::<HashSet<_>>();

    let mut reads = HashSet::new();
    for expression in hir.expressions() {
        if let Some(ExpressionResolution::DynamicCall(DynamicCallCallee::Value(value))) =
            expression.resolution
        {
            reads.insert(value);
        }
        let root = hir
            .value_path(expression.id)
            .and_then(|(root, _)| root)
            .or_else(|| {
                hir.call(expression.id).and_then(|call| {
                    call.receiver()
                        .and_then(|receiver| receiver.path().map(|(root, _)| root))
                })
            });
        if let Some(value) = root.and_then(|root| root.source_value()) {
            reads.insert(value);
        }
    }

    let mut writes = HashMap::<ast::ValueId, Vec<ast::Span>>::new();
    for body in hir.function_bodies() {
        collect_assignment_usage(&body.body, &mut reads, &mut writes);
    }
    for body in hir.action_bodies() {
        collect_assignment_usage(&body.body, &mut reads, &mut writes);
    }

    let mut diagnostics = collector
        .bindings
        .into_iter()
        .filter(|binding| !binding.name.starts_with('_') && !reads.contains(&binding.id))
        .map(|binding| {
            let mut replacement = format!("_{}", binding.name);
            while occupied_names.contains(&replacement) {
                replacement.insert(0, '_');
            }
            let mut edits = vec![TextEdit {
                span: binding.name_span,
                replacement: replacement.clone(),
            }];
            edits.extend(
                writes
                    .get(&binding.id)
                    .into_iter()
                    .flatten()
                    .map(|span| TextEdit {
                        span: ast::Span {
                            start: span.start,
                            end: span.start + binding.name.len(),
                        },
                        replacement: replacement.clone(),
                    }),
            );
            Diagnostic::warning(
                DiagnosticCode::UnusedBinding,
                format!("unused {} `{}`", binding.kind.description(), binding.name),
                binding.name_span,
            )
            .with_primary_label("this binding is never read")
            .with_note("prefix the name with `_` to indicate that this is intentional")
            .with_fix(DiagnosticFix {
                title: format!("rename `{}` to `{replacement}`", binding.name),
                applicability: FixApplicability::MachineApplicable,
                edits,
            })
        })
        .collect::<Vec<_>>();
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    diagnostics
}

fn collect_assignment_usage(
    block: &TypedBlock,
    reads: &mut HashSet<ast::ValueId>,
    writes: &mut HashMap<ast::ValueId, Vec<ast::Span>>,
) {
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Assign { assignment, op, .. } => {
                writes
                    .entry(assignment.target)
                    .or_default()
                    .push(assignment.span);
                if op.is_some() {
                    reads.insert(assignment.target);
                }
            }
            TypedStatementKind::If {
                then_block,
                else_block,
                ..
            } => {
                collect_assignment_usage(then_block, reads, writes);
                if let Some(else_block) = else_block {
                    collect_assignment_usage(else_block, reads, writes);
                }
            }
            TypedStatementKind::While { body, .. } | TypedStatementKind::For { body, .. } => {
                collect_assignment_usage(body, reads, writes);
            }
            TypedStatementKind::Variable { .. }
            | TypedStatementKind::StateAssign { .. }
            | TypedStatementKind::IndexAssign { .. }
            | TypedStatementKind::Suspend { .. }
            | TypedStatementKind::Expression(_) => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DeclarationWorkItem {
    Global(ast::ValueId),
    Function(ast::FunctionId),
}

struct DeclarationDependencyCollector<'a> {
    semantics: &'a SemanticModel,
    dependencies: HashSet<DeclarationWorkItem>,
    types: HashSet<crate::types::TypeId>,
    observed_settings: HashSet<ast::ValueId>,
    literal_setting_keys: HashSet<String>,
    has_dynamic_setting_lookup: bool,
    observed_record_fields: HashSet<ast::RecordFieldId>,
    observed_enum_variants: HashSet<ast::EnumVariantId>,
    fully_observed_types: HashSet<crate::types::TypeId>,
    capability_calls: HashSet<(crate::types::TypeId, StdlibItemId)>,
}

impl<'a> DeclarationDependencyCollector<'a> {
    fn new(semantics: &'a SemanticModel) -> Self {
        Self {
            semantics,
            dependencies: HashSet::new(),
            types: HashSet::new(),
            observed_settings: HashSet::new(),
            literal_setting_keys: HashSet::new(),
            has_dynamic_setting_lookup: false,
            observed_record_fields: HashSet::new(),
            observed_enum_variants: HashSet::new(),
            fully_observed_types: HashSet::new(),
            capability_calls: HashSet::new(),
        }
    }

    fn expand_capability_dependencies(
        &mut self,
        capabilities: &CapabilityAnalysis,
        semantics: &SemanticModel,
    ) {
        for (receiver, requirement) in std::mem::take(&mut self.capability_calls) {
            let dependencies = capabilities.method_dependencies(receiver, requirement, semantics);
            self.dependencies.extend(
                dependencies
                    .source_functions
                    .into_iter()
                    .map(DeclarationWorkItem::Function),
            );
            self.fully_observed_types
                .extend(dependencies.derived_aggregates);
        }
    }
}

impl TypedVisitor for DeclarationDependencyCollector<'_> {
    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        self.types.insert(expression.ty);
        self.capability_calls.extend(
            hir::implicit_display_types(expression, program, self.semantics)
                .into_iter()
                .map(|ty| (ty, StdlibItemId::DisplayToString)),
        );
        let mut observe_members = |members: &[ResolvedMember]| {
            self.observed_settings
                .extend(members.iter().filter_map(|member| match member {
                    ResolvedMember::SettingField(setting) => Some(*setting),
                    ResolvedMember::StateField(_)
                    | ResolvedMember::RecordField(_)
                    | ResolvedMember::StandardField(_) => None,
                }));
            self.observed_record_fields
                .extend(members.iter().filter_map(|member| match member {
                    ResolvedMember::RecordField(field) => Some(*field),
                    ResolvedMember::StateField(_)
                    | ResolvedMember::SettingField(_)
                    | ResolvedMember::StandardField(_) => None,
                }));
        };
        match &expression.resolution {
            Some(ExpressionResolution::ValuePath { members, .. })
            | Some(ExpressionResolution::Member { members }) => observe_members(members),
            Some(ExpressionResolution::Call(call)) => {
                if let Some(receiver) = call.receiver() {
                    observe_members(receiver.members());
                }
            }
            Some(ExpressionResolution::DynamicCall(_)) => {}
            Some(ExpressionResolution::EnumConstructor {
                variant: ResolvedEnumVariantId::Source(variant),
            }) => {
                self.observed_enum_variants.insert(*variant);
            }
            Some(ExpressionResolution::RecordLiteral { .. })
            | Some(ExpressionResolution::EnumConstructor {
                variant: ResolvedEnumVariantId::Standard(_),
            })
            | None => {}
        }
        if let TypedExpressionKind::Binary {
            op: ast::BinaryOp::Eq | ast::BinaryOp::Ne,
            left,
            ..
        } = expression.kind
        {
            self.fully_observed_types.insert(
                program
                    .expression(left)
                    .expect("binary operand belongs to typed HIR")
                    .ty,
            );
        }
        let resolved_root = program
            .value_path(expression.id)
            .and_then(|(root, _)| root)
            .or_else(|| {
                program.call(expression.id).and_then(|call| {
                    call.receiver()
                        .and_then(|receiver| receiver.path().map(|(root, _)| root))
                })
            });
        let source_value = resolved_root.and_then(ResolvedValue::source_value);
        if let Some(value) = source_value {
            self.dependencies.insert(DeclarationWorkItem::Global(value));
        }

        if let Some(ResolvedValue::Setting(setting) | ResolvedValue::OldSetting(setting)) =
            resolved_root
        {
            self.observed_settings.insert(setting);
        }

        if let (
            Some(ResolvedCall::StandardLibrary { item, .. }),
            TypedExpressionKind::Call { arguments, .. },
        ) = (program.call(expression.id), &expression.kind)
            && matches!(
                *item,
                crate::stdlib::StdlibItemId::SettingsViewEnabled
                    | crate::stdlib::StdlibItemId::SettingsViewContains
            )
            && let Some(argument) = arguments.first()
        {
            match &program
                .expression(*argument)
                .expect("call argument belongs to typed HIR")
                .kind
            {
                TypedExpressionKind::String(key) => {
                    self.literal_setting_keys.insert(key.clone());
                }
                _ => self.has_dynamic_setting_lookup = true,
            }
        }

        if let Some(
            ResolvedCall::UserFunction { function, .. } | ResolvedCall::UserMethod { function, .. },
        ) = program.call(expression.id)
        {
            self.dependencies
                .insert(DeclarationWorkItem::Function(*function));
        }

        if let Some(ResolvedCall::StandardLibrary {
            item,
            receiver_type: Some(receiver),
            ..
        }) = program.call(expression.id)
        {
            let declaration = program.standard_library().item(*item);
            if declaration.implementation == Implementation::CapabilityRequirement
                && matches!(declaration.owner, StdlibOwner::Capability(_))
            {
                self.capability_calls.insert((*receiver, *item));
            }
        }

        hir::walk_typed_expression(self, expression, program);
    }

    fn visit_match_arm(&mut self, arm: &TypedMatchArm, program: &TypedProgram) {
        if let Some(ResolvedEnumVariantId::Source(variant)) = arm.resolution.variant {
            self.observed_enum_variants.insert(variant);
        }
        hir::walk_typed_match_arm(self, arm, program);
    }
}

/// Finds source declarations that can be reached from compiler-invoked code.
///
/// Unlike backend reachability, global initializers are not unconditional
/// roots here. An initializer becomes reachable only after its global is read,
/// which lets this user-facing analysis diagnose a dead global together with
/// helper declarations used exclusively by that initializer.
fn validate_unused_declarations(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &CapabilityAnalysis,
) -> Vec<Diagnostic> {
    let globals = syntax
        .globals
        .iter()
        .map(|global| (global.id, global))
        .collect::<HashMap<_, _>>();
    let initializers = hir
        .global_initializers()
        .map(|initializer| (initializer.value, initializer.expression))
        .collect::<HashMap<_, _>>();
    let functions = hir
        .function_bodies()
        .map(|body| (body.function.function, body))
        .collect::<HashMap<_, _>>();

    let mut reachable = HashSet::new();
    let mut reachable_types = HashSet::new();
    let mut observed_settings = HashSet::new();
    let mut literal_setting_keys = HashSet::new();
    let mut has_dynamic_setting_lookup = false;
    let mut observed_record_fields = HashSet::new();
    let mut observed_enum_variants = HashSet::new();
    let mut fully_observed_types = HashSet::new();
    let mut pending = VecDeque::new();
    let mut roots = DeclarationDependencyCollector::new(semantics);
    for action in hir.action_bodies() {
        roots.visit_block(&action.body, hir);
    }
    for (_, expression) in hir.state_sources() {
        roots.visit_expression(
            hir.expression(expression)
                .expect("state source belongs to typed HIR"),
            hir,
        );
    }
    roots.expand_capability_dependencies(capabilities, semantics);
    reachable_types.extend(roots.types.iter().copied());
    observed_settings.extend(roots.observed_settings.iter().copied());
    literal_setting_keys.extend(roots.literal_setting_keys.iter().cloned());
    has_dynamic_setting_lookup |= roots.has_dynamic_setting_lookup;
    observed_record_fields.extend(roots.observed_record_fields.iter().copied());
    observed_enum_variants.extend(roots.observed_enum_variants.iter().copied());
    fully_observed_types.extend(roots.fully_observed_types.iter().copied());
    pending.extend(roots.dependencies);

    // State and settings are host-visible declarations even when user code
    // never names their snapshots. Their complete value types therefore seed
    // nominal type reachability.
    if let Some(state) = &syntax.state {
        reachable_types.extend(
            state
                .all_fields()
                .filter_map(|field| semantics.value_type(field.id)),
        );
    }
    reachable_types.extend(
        syntax
            .settings
            .iter()
            .filter_map(|setting| semantics.value_type(setting.id)),
    );
    for setting in &syntax.settings {
        if let ast::SettingKind::Choice { options, .. } = &setting.kind {
            if let Some(default) = hir.setting_choice_default(setting.id) {
                observed_enum_variants.insert(default);
            }
            observed_enum_variants.extend(
                options
                    .iter()
                    .filter_map(|option| hir.setting_choice_option(option.id)),
            );
        }
    }

    while let Some(item) = pending.pop_front() {
        if !reachable.insert(item) {
            continue;
        }

        let mut collector = DeclarationDependencyCollector::new(semantics);
        match item {
            DeclarationWorkItem::Global(value) => {
                let Some(expression) = initializers.get(&value).copied() else {
                    // State, setting, and local values are ordinary semantic
                    // value roots but are not declaration work items.
                    continue;
                };
                collector.visit_expression(
                    hir.expression(expression)
                        .expect("global initializer belongs to typed HIR"),
                    hir,
                );
                if let Some(ty) = semantics.value_type(value) {
                    reachable_types.insert(ty);
                }
            }
            DeclarationWorkItem::Function(function) => {
                let Some(body) = functions.get(&function) else {
                    // Hidden standard-library source bodies are catalog-owned,
                    // not user declarations diagnosed by this pass.
                    continue;
                };
                collector.visit_block(&body.body, hir);
                reachable_types
                    .extend(semantics.function_parameter_types(function).iter().copied());
                if let Some(result) = semantics.function_result(function) {
                    reachable_types.insert(result);
                }
            }
        }
        collector.expand_capability_dependencies(capabilities, semantics);
        reachable_types.extend(collector.types.iter().copied());
        observed_settings.extend(collector.observed_settings.iter().copied());
        literal_setting_keys.extend(collector.literal_setting_keys.iter().cloned());
        has_dynamic_setting_lookup |= collector.has_dynamic_setting_lookup;
        observed_record_fields.extend(collector.observed_record_fields.iter().copied());
        observed_enum_variants.extend(collector.observed_enum_variants.iter().copied());
        fully_observed_types.extend(collector.fully_observed_types.iter().copied());
        pending.extend(collector.dependencies);
    }

    let (reachable_records, reachable_enums) =
        expand_reachable_nominal_types(&mut reachable_types, syntax, semantics);
    expand_fully_observed_types(
        fully_observed_types,
        syntax,
        semantics,
        &mut observed_record_fields,
        &mut observed_enum_variants,
    );

    let mut diagnostics = Vec::new();
    if !has_dynamic_setting_lookup {
        observed_settings.extend(
            syntax
                .settings
                .iter()
                .filter(|setting| literal_setting_keys.contains(setting.runtime_key()))
                .map(|setting| setting.id),
        );
        for setting in syntax.settings.iter().filter(|setting| {
            setting.source_visible
                && !matches!(setting.kind, ast::SettingKind::Title { .. })
                && !setting.name.starts_with('_')
                && !observed_settings.contains(&setting.id)
        }) {
            let replacement = if setting.external_key.is_some() {
                format!("_{}", setting.name)
            } else {
                format!(
                    "_{} key {}",
                    setting.name,
                    quote_string_literal(&setting.name)
                )
            };
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedMember,
                    format!("unused setting `{}`", setting.name),
                    setting.name_span,
                )
                .with_primary_label("this setting is never read from reachable code")
                .with_note(
                    "a declared setting cannot affect script behavior until its value is read",
                )
                .with_note("prefix the source name with `_` to indicate that this is intentional")
                .with_fix(DiagnosticFix {
                    title: format!("rename `{}` to `_{}`", setting.name, setting.name),
                    applicability: FixApplicability::MachineApplicable,
                    edits: vec![TextEdit {
                        span: setting.name_span,
                        replacement,
                    }],
                }),
            );
        }
    }
    for global in globals.values() {
        if !global.name.starts_with('_')
            && !reachable.contains(&DeclarationWorkItem::Global(global.id))
        {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused global `{}`", global.name),
                    global.name_span,
                )
                .with_primary_label("this global is never read from reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        }
    }
    for function in syntax.functions.iter().filter(|function| {
        functions.contains_key(&function.id)
            && !function.name.starts_with('_')
            && !reachable.contains(&DeclarationWorkItem::Function(function.id))
    }) {
        diagnostics.push(
            Diagnostic::warning(
                DiagnosticCode::UnusedDeclaration,
                format!("unused function `{}`", function.name),
                function.name_span,
            )
            .with_primary_label("this function is never called from reachable code")
            .with_note("prefix the name with `_` to indicate that this is intentional"),
        );
    }
    for record in &syntax.records {
        if !record.name.starts_with('_') && !reachable_records.contains(&record.id) {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused record `{}`", record.name),
                    record.name_span,
                )
                .with_primary_label("this record is never used by reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        }
    }
    for enumeration in &syntax.enums {
        if !enumeration.name.starts_with('_') && !reachable_enums.contains(&enumeration.id) {
            diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::UnusedDeclaration,
                    format!("unused enum `{}`", enumeration.name),
                    enumeration.name_span,
                )
                .with_primary_label("this enum is never used by reachable code")
                .with_note("prefix the name with `_` to indicate that this is intentional"),
            );
        }
    }
    for record in syntax
        .records
        .iter()
        .filter(|record| reachable_records.contains(&record.id))
    {
        for field in &record.fields {
            if !field.name.starts_with('_') && !observed_record_fields.contains(&field.id) {
                diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::UnusedMember,
                        format!("unused record field `{}.{}`", record.name, field.name),
                        field.name_span,
                    )
                    .with_primary_label("this field is never read from reachable code")
                    .with_note("constructing or deserializing a record does not read its fields")
                    .with_note(
                        "prefix the field name with `_` to indicate that this is intentional",
                    ),
                );
            }
        }
    }
    for enumeration in syntax
        .enums
        .iter()
        .filter(|enumeration| reachable_enums.contains(&enumeration.id))
    {
        for variant in &enumeration.variants {
            if !variant.name.starts_with('_') && !observed_enum_variants.contains(&variant.id) {
                diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::UnusedMember,
                        format!(
                            "unused enum variant `{}.{}`",
                            enumeration.name, variant.name
                        ),
                        variant.name_span,
                    )
                    .with_primary_label(
                        "this variant is never constructed or matched by reachable code",
                    )
                    .with_note(
                        "prefix the variant name with `_` to indicate that this is intentional",
                    ),
                );
            }
        }
    }
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    diagnostics
}

fn quote_string_literal(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\'' => quoted.push_str("\\'"),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn expand_fully_observed_types(
    roots: HashSet<crate::types::TypeId>,
    syntax: &Program,
    semantics: &SemanticModel,
    observed_record_fields: &mut HashSet<ast::RecordFieldId>,
    observed_enum_variants: &mut HashSet<ast::EnumVariantId>,
) {
    let records = syntax
        .records
        .iter()
        .map(|record| (record.id, record))
        .collect::<HashMap<_, _>>();
    let enums = syntax
        .enums
        .iter()
        .map(|enumeration| (enumeration.id, enumeration))
        .collect::<HashMap<_, _>>();
    let mut pending = roots.into_iter().collect::<VecDeque<_>>();
    let mut expanded = HashSet::new();

    while let Some(ty) = pending.pop_front() {
        if !expanded.insert(ty) {
            continue;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => {}
            TypeKind::StateSnapshot => {
                if let Some(state) = &syntax.state {
                    pending.extend(
                        state
                            .all_fields()
                            .filter_map(|field| semantics.value_type(field.id)),
                    );
                }
            }
            TypeKind::SettingsView => pending.extend(
                syntax
                    .settings
                    .iter()
                    .filter_map(|setting| semantics.value_type(setting.id)),
            ),
            TypeKind::Record(record) => {
                if let Some(record) = records.get(record) {
                    for field in &record.fields {
                        observed_record_fields.insert(field.id);
                        if let Some(ty) = semantics.record_field_type(field.id) {
                            pending.push_back(ty);
                        }
                    }
                }
            }
            TypeKind::Enum(enumeration) => {
                if let Some(enumeration) = enums.get(enumeration) {
                    for variant in &enumeration.variants {
                        observed_enum_variants.insert(variant.id);
                        if let Some(ty) = semantics.enum_variant_payload(variant.id) {
                            pending.push_back(ty);
                        }
                    }
                }
            }
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Async { value: element, .. }
            | TypeKind::Range { bound: element, .. } => pending.push_back(*element),
            TypeKind::Application { arguments, .. } => {
                pending.extend(arguments.iter().copied());
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                pending.extend(parameters.iter().copied());
                pending.push_back(*result);
            }
            TypeKind::Builtin(_) | TypeKind::Standard(_) | TypeKind::GenericParameter { .. } => {}
        }
    }
}

fn expand_reachable_nominal_types(
    roots: &mut HashSet<crate::types::TypeId>,
    syntax: &Program,
    semantics: &SemanticModel,
) -> (HashSet<ast::RecordId>, HashSet<ast::EnumId>) {
    let records = syntax
        .records
        .iter()
        .map(|record| (record.id, record))
        .collect::<HashMap<_, _>>();
    let enums = syntax
        .enums
        .iter()
        .map(|enumeration| (enumeration.id, enumeration))
        .collect::<HashMap<_, _>>();
    let mut pending = roots.iter().copied().collect::<VecDeque<_>>();
    let mut expanded = HashSet::new();
    let mut reachable_records = HashSet::new();
    let mut reachable_enums = HashSet::new();

    while let Some(ty) = pending.pop_front() {
        if !expanded.insert(ty) {
            continue;
        }
        match semantics.types().kind(ty) {
            TypeKind::Error => {}
            TypeKind::StateSnapshot => {
                if let Some(state) = &syntax.state {
                    pending.extend(
                        state
                            .all_fields()
                            .filter_map(|field| semantics.value_type(field.id)),
                    );
                }
            }
            TypeKind::SettingsView => pending.extend(
                syntax
                    .settings
                    .iter()
                    .filter_map(|setting| semantics.value_type(setting.id)),
            ),
            TypeKind::Record(record) => {
                reachable_records.insert(*record);
                if let Some(record) = records.get(record) {
                    pending.extend(
                        record
                            .fields
                            .iter()
                            .filter_map(|field| semantics.record_field_type(field.id)),
                    );
                }
            }
            TypeKind::Enum(enumeration) => {
                reachable_enums.insert(*enumeration);
                if let Some(enumeration) = enums.get(enumeration) {
                    pending.extend(
                        enumeration
                            .variants
                            .iter()
                            .filter_map(|variant| semantics.enum_variant_payload(variant.id)),
                    );
                }
            }
            TypeKind::Array { element, .. }
            | TypeKind::Set { element, .. }
            | TypeKind::Option { value: element, .. }
            | TypeKind::Result { value: element, .. }
            | TypeKind::Async { value: element, .. }
            | TypeKind::Range { bound: element, .. } => pending.push_back(*element),
            TypeKind::Application { arguments, .. } => {
                pending.extend(arguments.iter().copied());
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                pending.extend(parameters.iter().copied());
                pending.push_back(*result);
            }
            TypeKind::Builtin(_) | TypeKind::Standard(_) | TypeKind::GenericParameter { .. } => {}
        }
    }

    roots.extend(expanded);
    (reachable_records, reachable_enums)
}

fn validate_must_use(
    standard_library: &StandardLibrary,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for body in hir.function_bodies() {
        validate_must_use_block(
            standard_library,
            hir,
            semantics,
            &body.body,
            &mut diagnostics,
        );
    }
    for body in hir.action_bodies() {
        validate_must_use_block(
            standard_library,
            hir,
            semantics,
            &body.body,
            &mut diagnostics,
        );
    }
    diagnostics
}

fn validate_must_use_block(
    standard_library: &StandardLibrary,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    block: &TypedBlock,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Expression(expression_id) => {
                let expression = hir
                    .expression(*expression_id)
                    .expect("typed statement expressions belong to typed HIR");
                let callable = hir.call(*expression_id).and_then(|call| {
                    let crate::semantic::ResolvedCall::StandardLibrary { item, .. } = call else {
                        return None;
                    };
                    let item = standard_library.item(*item);
                    standard_library
                        .must_use(item.id)
                        .map(|reason| (item.qualified_name, reason))
                });
                let constructed = match semantics.types().kind(expression.ty) {
                    TypeKind::Option { .. } => Some(StdlibTypeConstructorId::Option),
                    TypeKind::Result { .. } => Some(StdlibTypeConstructorId::Result),
                    _ => None,
                }
                .and_then(|constructor| {
                    let declaration = standard_library.type_constructor(constructor);
                    declaration
                        .must_use
                        .map(|reason| (declaration.name, reason))
                })
                .or_else(|| {
                    matches!(semantics.types().kind(expression.ty), TypeKind::Async { .. })
                        .then_some((
                            "async operation",
                            "Await the future or store it for later; discarding it means the operation is never polled.",
                        ))
                });
                if let Some((name, reason)) = callable.or(constructed) {
                    diagnostics.push(
                        Diagnostic::warning(
                            DiagnosticCode::MustUse,
                            format!("unused result of `{name}`"),
                            expression.span,
                        )
                        .with_primary_label("this returned value is discarded")
                        .with_note(reason),
                    );
                }
            }
            TypedStatementKind::If {
                then_block,
                else_block,
                ..
            } => {
                validate_must_use_block(standard_library, hir, semantics, then_block, diagnostics);
                if let Some(else_block) = else_block {
                    validate_must_use_block(
                        standard_library,
                        hir,
                        semantics,
                        else_block,
                        diagnostics,
                    );
                }
            }
            TypedStatementKind::While { body, .. } | TypedStatementKind::For { body, .. } => {
                validate_must_use_block(standard_library, hir, semantics, body, diagnostics);
            }
            TypedStatementKind::Variable { .. }
            | TypedStatementKind::Assign { .. }
            | TypedStatementKind::StateAssign { .. }
            | TypedStatementKind::IndexAssign { .. }
            | TypedStatementKind::Suspend { .. } => {}
        }
    }
}

const MAX_FUNCTION_INSTANCES: usize = 256;
const MAX_INSTANCE_DEPTH: usize = 64;

fn validate_function_instances(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<Diagnostic> {
    use std::collections::{BTreeSet, HashMap};

    let mut calls = HashMap::<Option<ast::FunctionId>, Vec<_>>::new();
    for expression in hir.all_expressions() {
        let Some(call) = hir.call(expression.id) else {
            continue;
        };
        let owner = syntax
            .functions
            .iter()
            .filter(|function| {
                function.body.span.start <= expression.span.start
                    && expression.span.end <= function.body.span.end
            })
            .min_by_key(|function| function.body.span.end - function.body.span.start)
            .map(|function| function.id);
        calls
            .entry(owner)
            .or_default()
            .push((call, expression.span));
    }

    let to_instance = |call: &ResolvedCall| match call {
        ResolvedCall::UserFunction {
            function,
            type_arguments,
            signature,
        }
        | ResolvedCall::UserMethod {
            function,
            type_arguments,
            signature,
            ..
        } => Some(FunctionInstance {
            function: *function,
            type_arguments: type_arguments.clone(),
            signature: signature.clone(),
        }),
        ResolvedCall::StandardLibrary { .. }
        | ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::IteratorItem { .. }
        | ResolvedCall::ResultSuccess { .. } => None,
    };

    let mut pending = calls
        .get(&None)
        .into_iter()
        .flatten()
        .filter_map(|(call, span)| {
            to_instance(call).map(|instance| {
                let depth = usize::from(!instance.type_arguments.is_empty());
                (instance, depth, *span)
            })
        })
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut generic_instances = 0usize;
    while let Some((instance, depth, span)) = pending.pop() {
        if !visited.insert(instance.clone()) {
            continue;
        }
        generic_instances += usize::from(!instance.type_arguments.is_empty());
        if generic_instances > MAX_FUNCTION_INSTANCES {
            return vec![Diagnostic::semantic(
                format!(
                    "generic function expansion exceeds the limit of {MAX_FUNCTION_INSTANCES} concrete instances"
                ),
                span,
            )];
        }
        if depth > MAX_INSTANCE_DEPTH {
            return vec![Diagnostic::semantic(
                format!(
                    "generic function expansion exceeds the recursion-depth limit of {MAX_INSTANCE_DEPTH}"
                ),
                span,
            )];
        }
        for (call, call_span) in calls.get(&Some(instance.function)).into_iter().flatten() {
            if let Some(called) = to_instance(call) {
                let called = semantics.specialize_function_instance(&instance, &called);
                let called_depth = depth + usize::from(!called.type_arguments.is_empty());
                pending.push((called, called_depth, *call_span));
            }
        }
    }
    Vec::new()
}
