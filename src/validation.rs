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
    semantic::{ResolvedCall, SemanticModel},
    stdlib::{StandardLibrary, StdlibCapabilityId},
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
            if matches!(field.source, StateSource::Pointer(_)) {
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
