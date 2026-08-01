//! Operational-effect inference over resolved user-function call graphs.

use crate::{
    ast::{ActionKind, FunctionId, Span},
    hir::{self, TypedExpression, TypedProgram, TypedVisitor},
    semantic::ResolvedCall,
    stdlib::{
        Availability, CancellationKind, Effect, EffectSet, OperationMetadata, SuspensionKind,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionOperationSemantics {
    pub effects: Vec<Effect>,
    pub requires_attached_process: bool,
    pub availability: Availability,
    pub suspension: SuspensionKind,
    pub cancellation: CancellationKind,
}

impl Default for FunctionOperationSemantics {
    fn default() -> Self {
        function_semantics(Vec::new(), Availability::Everywhere)
    }
}

impl FunctionOperationSemantics {
    pub fn metadata(&self) -> OperationMetadata {
        OperationMetadata {
            effects: self
                .effects
                .iter()
                .fold(EffectSet::none(), |effects, effect| effects.with(*effect)),
            availability: self.availability,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAnalysis {
    functions: Vec<FunctionOperationSemantics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetachedCallViolation {
    pub expression_span: Span,
    pub function: Option<FunctionId>,
    pub standard_library_name: Option<&'static str>,
}

struct CallFacts {
    effects: Vec<Effect>,
    callees: Vec<FunctionId>,
    availability: Availability,
}

impl Default for CallFacts {
    fn default() -> Self {
        Self {
            effects: Vec::new(),
            callees: Vec::new(),
            availability: Availability::Everywhere,
        }
    }
}

struct CallCollector<'a> {
    facts: &'a mut CallFacts,
}

impl TypedVisitor for CallCollector<'_> {
    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        match program.call(expression.id) {
            Some(ResolvedCall::StandardLibrary { item, .. }) => {
                if let Some(function) = program.library_function(*item) {
                    self.facts.callees.push(function);
                } else {
                    let metadata = program.standard_library().operation_metadata(*item);
                    self.facts.effects.extend(metadata.effects.iter().copied());
                    self.facts.availability =
                        merge_availability(self.facts.availability, metadata.availability);
                }
            }
            Some(
                ResolvedCall::UserFunction { function, .. }
                | ResolvedCall::UserMethod { function, .. },
            ) => self.facts.callees.push(*function),
            Some(
                ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::ResultSuccess { .. },
            )
            | None => {}
        }
        hir::walk_typed_expression(self, expression, program);
    }
}

impl OperationAnalysis {
    pub fn infer(program: &TypedProgram) -> Self {
        let function_count = program
            .all_function_bodies()
            .map(|body| body.function.function.index() + 1)
            .max()
            .unwrap_or(0);
        let mut direct = (0..function_count)
            .map(|_| CallFacts::default())
            .collect::<Vec<_>>();
        for function in program.all_function_bodies() {
            CallCollector {
                facts: &mut direct[function.function.function.index()],
            }
            .visit_block(&function.body, program);
        }

        let mut functions = direct
            .iter()
            .map(|facts| function_semantics(facts.effects.clone(), facts.availability))
            .collect::<Vec<_>>();
        loop {
            let mut changed = false;
            for (index, facts) in direct.iter().enumerate() {
                let mut effects = facts.effects.clone();
                let mut availability = facts.availability;
                for callee in &facts.callees {
                    if let Some(callee) = functions.get(callee.index()) {
                        effects.extend_from_slice(&callee.effects);
                        availability = merge_availability(availability, callee.availability);
                    }
                }
                let inferred = function_semantics(effects, availability);
                if inferred != functions[index] {
                    functions[index] = inferred;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Self { functions }
    }

    pub fn function(&self, function: FunctionId) -> FunctionOperationSemantics {
        self.functions
            .get(function.index())
            .cloned()
            .unwrap_or_default()
    }

    pub fn detached_call_violations(&self, program: &TypedProgram) -> Vec<DetachedCallViolation> {
        let Some(body) = program.action_body(ActionKind::OnDetached) else {
            return Vec::new();
        };
        struct Validator<'a> {
            analysis: &'a OperationAnalysis,
            violations: Vec<DetachedCallViolation>,
        }
        impl TypedVisitor for Validator<'_> {
            fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
                let violation = match program.call(expression.id) {
                    Some(ResolvedCall::StandardLibrary { item, .. }) => {
                        let item = program.standard_library().item(*item);
                        let requires_attached_process = program
                            .library_function(item.id)
                            .map(|function| {
                                self.analysis.function(function).requires_attached_process
                            })
                            .unwrap_or_else(|| {
                                program
                                    .standard_library()
                                    .operation_semantics(item.id)
                                    .requires_attached_process
                            });
                        requires_attached_process.then_some(DetachedCallViolation {
                            expression_span: expression.span,
                            function: None,
                            standard_library_name: Some(item.qualified_name),
                        })
                    }
                    Some(
                        ResolvedCall::UserFunction { function, .. }
                        | ResolvedCall::UserMethod { function, .. },
                    ) => self
                        .analysis
                        .function(*function)
                        .requires_attached_process
                        .then_some(DetachedCallViolation {
                            expression_span: expression.span,
                            function: Some(*function),
                            standard_library_name: None,
                        }),
                    Some(
                        ResolvedCall::ResultError { .. }
                        | ResolvedCall::OptionSome { .. }
                        | ResolvedCall::ResultSuccess { .. },
                    )
                    | None => None,
                };
                if let Some(violation) = violation {
                    self.violations.push(violation);
                }
                hir::walk_typed_expression(self, expression, program);
            }
        }

        let mut validator = Validator {
            analysis: self,
            violations: Vec::new(),
        };
        validator.visit_block(body, program);
        validator.violations
    }
}

fn function_semantics(
    mut effects: Vec<Effect>,
    availability: Availability,
) -> FunctionOperationSemantics {
    effects.sort_by_key(|effect| effect_order(*effect));
    effects.dedup();
    if effects.len() > 1 {
        effects.retain(|effect| *effect != Effect::Pure);
    }
    if effects.is_empty() {
        effects.push(Effect::Pure);
    }
    let metadata = OperationMetadata {
        effects: effects
            .iter()
            .fold(EffectSet::none(), |set, effect| set.with(*effect)),
        availability,
    };
    let operation = metadata.semantics();
    FunctionOperationSemantics {
        effects,
        requires_attached_process: operation.requires_attached_process,
        availability,
        suspension: operation.suspension,
        cancellation: operation.cancellation,
    }
}

const fn merge_availability(left: Availability, right: Availability) -> Availability {
    if matches!(left, Availability::OnAttach) || matches!(right, Availability::OnAttach) {
        Availability::OnAttach
    } else {
        Availability::Everywhere
    }
}

const fn effect_order(effect: Effect) -> u8 {
    match effect {
        Effect::Pure => 0,
        Effect::Allocates => 1,
        Effect::MutatesValue => 2,
        Effect::ReadsTimer => 3,
        Effect::ReadsProcess => 4,
        Effect::RequiresAttachedProcess => 5,
        Effect::Retryable => 6,
        Effect::Suspends => 7,
        Effect::CancelsOnProcessClose => 8,
        Effect::WritesTimer => 9,
        Effect::WritesRuntime => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_function_semantics_preserve_every_operational_dimension() {
        let inferred = function_semantics(
            vec![
                Effect::Pure,
                Effect::ReadsProcess,
                Effect::RequiresAttachedProcess,
                Effect::Suspends,
                Effect::CancelsOnProcessClose,
            ],
            Availability::OnAttach,
        );
        assert!(!inferred.effects.contains(&Effect::Pure));
        assert!(inferred.effects.contains(&Effect::ReadsProcess));
        assert!(inferred.requires_attached_process);
        assert_eq!(inferred.availability, Availability::OnAttach);
        assert_eq!(inferred.suspension, SuspensionKind::Suspends);
        assert_eq!(inferred.cancellation, CancellationKind::ProcessClose);
        assert_eq!(
            inferred.metadata().semantics().suspension,
            inferred.suspension
        );
    }
}
