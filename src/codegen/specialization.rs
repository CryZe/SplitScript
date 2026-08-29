//! Demand-driven materialization of concrete types used only inside generic
//! function templates.

use std::collections::{BTreeSet, HashMap};

use crate::{
    ast::{ConstructedTypeIdAllocator, ExprId},
    semantic::{FunctionInstance, SemanticModel},
    types::{
        ResolvedApplicationType, ResolvedArrayType, ResolvedAsyncType, ResolvedCallableType,
        ResolvedConstructedTypesMut, ResolvedOptionType, ResolvedRangeType, ResolvedResultType,
        ResolvedSetType, TypeId,
    },
    wasm_ir::{self, BodyOwner, Visitor},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn materialize(
    wasm: &wasm_ir::Program,
    program: &crate::ast::Program,
    capabilities: &crate::capabilities::CapabilityAnalysis,
    semantics: &mut SemanticModel,
    arrays: &mut Vec<ResolvedArrayType>,
    options: &mut Vec<ResolvedOptionType>,
    results: &mut Vec<ResolvedResultType>,
    asyncs: &mut Vec<ResolvedAsyncType>,
    callables: &mut Vec<ResolvedCallableType>,
    ranges: &mut Vec<ResolvedRangeType>,
    sets: &mut Vec<ResolvedSetType>,
    applications: &mut Vec<ResolvedApplicationType>,
) {
    let next = arrays
        .iter()
        .map(|ty| ty.id.index() as u32 + 1)
        .chain(options.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(results.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(asyncs.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(callables.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(ranges.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(sets.iter().map(|ty| ty.id.index() as u32 + 1))
        .chain(applications.iter().map(|ty| ty.id.index() as u32 + 1))
        .max()
        .unwrap_or_default();
    let mut ids = ConstructedTypeIdAllocator::starting_at(next);
    let mut constructed = ResolvedConstructedTypesMut {
        arrays,
        options,
        results,
        asyncs,
        callables,
        ranges,
        sets,
        applications,
    };
    let owners = expression_owners(wasm);
    let mut pending = owners
        .iter()
        .filter(|(_, owner)| owner.is_none())
        .filter_map(|(expression, _)| {
            called_function(
                &wasm.expression(*expression)?.kind,
                None,
                semantics,
                wasm.standard_library(),
                program,
                capabilities,
            )
        })
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();

    while let Some(instance) = pending.pop() {
        if !visited.insert(instance.clone()) {
            continue;
        }
        let body = wasm
            .body(BodyOwner::Function(instance.clone()))
            .expect("reachable calls have function templates");
        for local in &body.locals {
            materialize_type(semantics, &instance, local.ty, &mut ids, &mut constructed);
        }
        for (expression, owner) in &owners {
            if *owner != Some(instance.function) {
                continue;
            }
            let expression = wasm
                .expression(*expression)
                .expect("owned expressions belong to Wasm IR");
            materialize_expression_types(
                expression,
                &instance,
                semantics,
                &mut ids,
                &mut constructed,
            );
            if let Some(called) = called_function(
                &expression.kind,
                Some(&instance),
                semantics,
                wasm.standard_library(),
                program,
                capabilities,
            ) {
                pending.push(semantics.specialize_function_instance(&instance, &called));
            }
        }
    }
}

fn expression_owners(wasm: &wasm_ir::Program) -> HashMap<ExprId, Option<crate::ast::FunctionId>> {
    struct Collector<'a> {
        owner: Option<crate::ast::FunctionId>,
        owners: &'a mut HashMap<ExprId, Option<crate::ast::FunctionId>>,
    }
    impl Visitor for Collector<'_> {
        fn visit_expression(
            &mut self,
            expression: &wasm_ir::Expression,
            program: &wasm_ir::Program,
        ) {
            self.owners.insert(expression.id, self.owner);
            wasm_ir::walk_expression(self, expression, program);
        }
    }

    let mut owners = HashMap::new();
    for body in wasm.bodies() {
        let owner = match &body.owner {
            BodyOwner::Function(instance) => Some(instance.function),
            BodyOwner::Action(_) => None,
        };
        Collector {
            owner,
            owners: &mut owners,
        }
        .visit_block(&body.entry, wasm);
    }
    for expression in wasm.state_expressions() {
        Collector {
            owner: None,
            owners: &mut owners,
        }
        .visit_block(&expression.entry, wasm);
    }
    for transform in wasm.state_transforms() {
        Collector {
            owner: None,
            owners: &mut owners,
        }
        .visit_block(&transform.entry, wasm);
    }
    for initializer in wasm.global_initializer_plans() {
        Collector {
            owner: None,
            owners: &mut owners,
        }
        .visit_block(&initializer.entry, wasm);
    }
    owners
}

fn called_function(
    kind: &wasm_ir::ExpressionKind,
    owner: Option<&FunctionInstance>,
    semantics: &SemanticModel,
    library: &crate::stdlib::StandardLibrary,
    program: &crate::ast::Program,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<FunctionInstance> {
    let wasm_ir::ExpressionKind::Call { target, .. } = kind else {
        return None;
    };
    match target {
        wasm_ir::CallTarget::UserFunction { function }
        | wasm_ir::CallTarget::UserMethod { function, .. } => Some(function.clone()),
        target @ wasm_ir::CallTarget::LibraryOverload { .. } => {
            wasm_ir::resolve_library_overload(target, owner, semantics, library)
        }
        target @ wasm_ir::CallTarget::CapabilityRequirement { .. } => {
            let resolved = wasm_ir::resolve_capability_requirement(
                target,
                owner,
                program,
                semantics,
                library,
                capabilities,
            )?;
            match resolved {
                wasm_ir::CallTarget::UserFunction { function }
                | wasm_ir::CallTarget::UserMethod { function, .. } => Some(function),
                target @ wasm_ir::CallTarget::LibraryOverload { .. } => {
                    wasm_ir::resolve_library_overload(&target, None, semantics, library)
                }
                wasm_ir::CallTarget::Intrinsic { .. }
                | wasm_ir::CallTarget::DefaultDisplay { .. }
                | wasm_ir::CallTarget::ManagedSnapshot { .. }
                | wasm_ir::CallTarget::ManagedComponent { .. }
                | wasm_ir::CallTarget::ManagedInstances { .. }
                | wasm_ir::CallTarget::ResultError { .. }
                | wasm_ir::CallTarget::OptionSome { .. }
                | wasm_ir::CallTarget::IteratorItem { .. }
                | wasm_ir::CallTarget::ResultSuccess { .. } => None,
                wasm_ir::CallTarget::CapabilityRequirement { .. } => {
                    unreachable!("capability resolution is concrete")
                }
            }
        }
        wasm_ir::CallTarget::Intrinsic { .. }
        | wasm_ir::CallTarget::DefaultDisplay { .. }
        | wasm_ir::CallTarget::ManagedSnapshot { .. }
        | wasm_ir::CallTarget::ManagedComponent { .. }
        | wasm_ir::CallTarget::ManagedInstances { .. }
        | wasm_ir::CallTarget::ResultError { .. }
        | wasm_ir::CallTarget::OptionSome { .. }
        | wasm_ir::CallTarget::IteratorItem { .. }
        | wasm_ir::CallTarget::ResultSuccess { .. } => None,
    }
}

fn materialize_expression_types(
    expression: &wasm_ir::Expression,
    instance: &FunctionInstance,
    semantics: &mut SemanticModel,
    ids: &mut ConstructedTypeIdAllocator,
    constructed: &mut ResolvedConstructedTypesMut<'_>,
) {
    materialize_type(semantics, instance, expression.ty, ids, constructed);
    if let Some(conversion) = expression.conversion {
        for ty in [conversion.source, conversion.target] {
            materialize_type(semantics, instance, ty, ids, constructed);
        }
    }
    match &expression.kind {
        wasm_ir::ExpressionKind::InterpolatedString(parts) => {
            for source in parts.iter().filter_map(|part| match part {
                wasm_ir::InterpolatedPart::Expression {
                    string_conversion_source,
                    ..
                } => *string_conversion_source,
                wasm_ir::InterpolatedPart::Text(_) => None,
            }) {
                materialize_type(semantics, instance, source, ids, constructed);
            }
        }
        wasm_ir::ExpressionKind::Call { target, .. } => match target {
            wasm_ir::CallTarget::UserMethod { receiver_type, .. } => {
                materialize_type(semantics, instance, *receiver_type, ids, constructed);
            }
            wasm_ir::CallTarget::Intrinsic {
                type_arguments,
                receiver_type,
                ..
            } => {
                for ty in type_arguments.iter().copied().chain(*receiver_type) {
                    materialize_type(semantics, instance, ty, ids, constructed);
                }
            }
            wasm_ir::CallTarget::LibraryOverload {
                dispatch_type,
                receiver_type,
                ..
            } => {
                for ty in std::iter::once(*dispatch_type).chain(*receiver_type) {
                    materialize_type(semantics, instance, ty, ids, constructed);
                }
            }
            wasm_ir::CallTarget::CapabilityRequirement {
                receiver_type,
                signature,
                ..
            } => {
                for ty in std::iter::once(*receiver_type).chain(signature.iter().copied()) {
                    materialize_type(semantics, instance, ty, ids, constructed);
                }
            }
            wasm_ir::CallTarget::DefaultDisplay { receiver_type, .. } => {
                materialize_type(semantics, instance, *receiver_type, ids, constructed);
            }
            wasm_ir::CallTarget::ManagedSnapshot { receiver_type, .. } => {
                materialize_type(semantics, instance, *receiver_type, ids, constructed);
            }
            wasm_ir::CallTarget::ManagedComponent {
                receiver_type,
                helper_result,
                ..
            } => {
                materialize_type(semantics, instance, *receiver_type, ids, constructed);
                materialize_type(semantics, instance, *helper_result, ids, constructed);
            }
            wasm_ir::CallTarget::UserFunction { .. }
            | wasm_ir::CallTarget::ManagedInstances { .. }
            | wasm_ir::CallTarget::ResultError { .. }
            | wasm_ir::CallTarget::OptionSome { .. }
            | wasm_ir::CallTarget::IteratorItem { .. }
            | wasm_ir::CallTarget::ResultSuccess { .. } => {}
        },
        wasm_ir::ExpressionKind::Propagate { target, .. } => {
            materialize_type(semantics, instance, target.result(), ids, constructed);
        }
        _ => {}
    }
}

fn materialize_type(
    semantics: &mut SemanticModel,
    instance: &FunctionInstance,
    ty: TypeId,
    ids: &mut ConstructedTypeIdAllocator,
    constructed: &mut ResolvedConstructedTypesMut<'_>,
) {
    semantics.materialize_specialized_type(instance, ty, ids, constructed);
}
