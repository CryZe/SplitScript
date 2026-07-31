//! Operational-effect inference over resolved user-function call graphs.

use crate::{
    ast::{ActionKind, FunctionId, Span},
    hir::{self, TypedExpression, TypedProgram, TypedVisitor},
    semantic::ResolvedCall,
    stdlib::Effect,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionOperationSemantics {
    pub effects: Vec<Effect>,
    pub requires_attached_process: bool,
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

#[derive(Default)]
struct CallFacts {
    effects: Vec<Effect>,
    callees: Vec<FunctionId>,
}

struct CallCollector<'a> {
    facts: &'a mut CallFacts,
}

impl TypedVisitor for CallCollector<'_> {
    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        match program.call(expression.id) {
            Some(ResolvedCall::StandardLibrary { item, .. }) => {
                self.facts.effects.extend(
                    program
                        .standard_library()
                        .item(*item)
                        .effects
                        .iter()
                        .copied(),
                );
            }
            Some(
                ResolvedCall::UserFunction { function } | ResolvedCall::UserMethod { function, .. },
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
            .function_bodies()
            .map(|body| body.function.index() + 1)
            .max()
            .unwrap_or(0);
        let mut direct = (0..function_count)
            .map(|_| CallFacts::default())
            .collect::<Vec<_>>();
        for function in program.function_bodies() {
            CallCollector {
                facts: &mut direct[function.function.index()],
            }
            .visit_block(&function.body, program);
        }

        let mut functions = direct
            .iter()
            .map(|facts| function_semantics(facts.effects.clone()))
            .collect::<Vec<_>>();
        loop {
            let mut changed = false;
            for (index, facts) in direct.iter().enumerate() {
                let mut effects = facts.effects.clone();
                for callee in &facts.callees {
                    if let Some(callee) = functions.get(callee.index()) {
                        effects.extend_from_slice(&callee.effects);
                    }
                }
                let inferred = function_semantics(effects);
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
                        item.operation_semantics()
                            .requires_attached_process
                            .then_some(DetachedCallViolation {
                                expression_span: expression.span,
                                function: None,
                                standard_library_name: Some(item.qualified_name),
                            })
                    }
                    Some(
                        ResolvedCall::UserFunction { function }
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

fn function_semantics(mut effects: Vec<Effect>) -> FunctionOperationSemantics {
    effects.sort_by_key(|effect| effect_order(*effect));
    effects.dedup();
    if effects.len() > 1 {
        effects.retain(|effect| *effect != Effect::Pure);
    }
    if effects.is_empty() {
        effects.push(Effect::Pure);
    }
    FunctionOperationSemantics {
        requires_attached_process: effects.contains(&Effect::RequiresAttachedProcess),
        effects,
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
