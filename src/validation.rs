//! Post-type-check semantic validation.
//!
//! Type inference establishes resolved types and calls. This stage derives
//! operational/capability facts and reports constraints that require the
//! complete typed program. Strict compilation and editor recovery consume the
//! same product.

use crate::{
    Diagnostic,
    ast::{self, EnumDecl, Program, StateSource},
    capabilities::CapabilityAnalysis,
    effects::OperationAnalysis,
    hir::{TypedExpressionKind, TypedProgram},
    semantic::{FunctionInstance, ResolvedCall, SemanticModel},
    stdlib::{Implementation, StandardLibrary, StdlibCapabilityId},
};

pub(crate) struct ValidationOutput {
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
    let effects = OperationAnalysis::infer(hir);
    let capabilities = CapabilityAnalysis::build_with_library(
        &syntax.records,
        enum_types,
        semantics,
        standard_library.clone(),
    );
    let mut diagnostics = Vec::new();
    diagnostics.extend(validate_function_instances(syntax, hir, semantics));

    // The standalone standard-library bootstrap is the authority for catalog
    // metadata. Every ordinary compilation rechecks the same injected bodies
    // and verifies that their complete typed call graph still agrees with the
    // cached, user-independent result.
    if standard_library.source_body_operations_are_initialized() {
        for item in standard_library.items() {
            if !matches!(item.implementation, Implementation::LibraryBody { .. }) {
                continue;
            }
            let cataloged = standard_library.operation_metadata(item.id);
            let function = hir
                .library_function(item.id)
                .expect("validated source bodies have function identities");
            let inferred = effects.function(function).metadata();
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

    for violation in effects.detached_call_violations(hir) {
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
                "`{}` requires an attached process and is unavailable in `onDetached`",
                name.unwrap_or_else(|| "function".to_owned())
            ),
            violation.expression_span,
        ));
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
                        diagnostics.push(Diagnostic::semantic(
                            format!(
                                "`{:?}` does not satisfy {} for `{}`: {error}",
                                semantics.types().kind(*argument),
                                capability.name,
                                item.qualified_name,
                            ),
                            expression.span,
                        ));
                    }
                }
            }
        }
    }

    if let Some(state) = &syntax.state {
        for field in &state.fields {
            if matches!(
                field.source,
                StateSource::Pointer(ref path) if path.decoder.is_none()
            ) {
                let ty = semantics
                    .value_type(field.id)
                    .expect("checked state fields have semantic types");
                if let Err(error) =
                    capabilities.require(ty, StdlibCapabilityId::MemoryReadable, semantics)
                {
                    diagnostics.push(Diagnostic::semantic(error, field.span));
                }
            }
        }
    }

    ValidationOutput {
        capabilities,
        effects,
        diagnostics,
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
