//! Operational-effect inference over resolved user-function call graphs.

use crate::{
    ast::{ActionKind, FunctionId, Span},
    hir::{self, TypedExpression, TypedProgram, TypedVisitor},
    semantic::{ResolvedCall, SemanticModel},
    stdlib::{
        Availability, CancellationKind, Effect, EffectSet, OperationMetadata, SuspensionKind,
    },
    types::TypeKind,
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
    future_callees: Vec<FunctionId>,
    availability: Availability,
}

impl Default for CallFacts {
    fn default() -> Self {
        Self {
            effects: Vec::new(),
            callees: Vec::new(),
            future_callees: Vec::new(),
            availability: Availability::Everywhere,
        }
    }
}

struct CallCollector<'a> {
    facts: &'a mut CallFacts,
    semantics: &'a SemanticModel,
}

fn collect_call_facts(
    facts: &mut CallFacts,
    call: &ResolvedCall,
    program: &TypedProgram,
    creates_future: bool,
) {
    if creates_future {
        facts.effects.push(Effect::Allocates);
    }
    match call {
        ResolvedCall::StandardLibrary { item, .. } => {
            if let Some(function) = program.library_function(*item) {
                if creates_future {
                    facts.future_callees.push(function);
                } else {
                    facts.callees.push(function);
                }
            } else {
                let metadata = program.standard_library().operation_metadata(*item);
                if !creates_future {
                    facts.effects.extend(metadata.effects.iter().copied());
                }
                facts.availability = merge_availability(facts.availability, metadata.availability);
            }
        }
        ResolvedCall::UserFunction { function, .. } | ResolvedCall::UserMethod { function, .. } => {
            if creates_future {
                facts.future_callees.push(*function)
            } else {
                facts.callees.push(*function)
            }
        }
        ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::ResultSuccess { .. } => {}
    }
}

impl TypedVisitor for CallCollector<'_> {
    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        if let hir::TypedExpressionKind::Suspend { value, .. } = expression.kind {
            self.facts
                .effects
                .extend([Effect::Suspends, Effect::CancelsOnProcessClose]);
            self.facts.availability = Availability::OnAttach;
            if let Some(call) = program.call(value) {
                collect_call_facts(self.facts, call, program, false);
            }
        }
        if let Some(call) = program.call(expression.id) {
            let creates_future = matches!(
                self.semantics.types().kind(expression.ty),
                TypeKind::Async { .. }
            );
            collect_call_facts(self.facts, call, program, creates_future);
        }
        hir::walk_typed_expression(self, expression, program);
    }

    fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
        // `retry` is suspending control flow even when its operand is merely a
        // synchronous operation returning `T!`. Recording the statement
        // itself also makes asyncness an authored-body fact rather than an
        // accidental property of the callee's effect declaration.
        if matches!(statement.kind, hir::TypedStatementKind::Suspend { .. }) {
            self.facts
                .effects
                .extend([Effect::Suspends, Effect::CancelsOnProcessClose]);
            self.facts.availability = Availability::OnAttach;
        }
        if let hir::TypedStatementKind::Assign { assignment, .. } = &statement.kind
            && let Some(call) = &assignment.operator
        {
            collect_call_facts(self.facts, call, program, false);
        }
        hir::walk_typed_statement(self, statement, program);
    }
}

impl OperationAnalysis {
    pub fn infer(program: &TypedProgram, semantics: &SemanticModel) -> Self {
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
                semantics,
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
                for callee in &facts.future_callees {
                    if let Some(callee) = functions.get(callee.index()) {
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
        impl Validator<'_> {
            fn violation(
                &self,
                call: &ResolvedCall,
                span: Span,
                program: &TypedProgram,
            ) -> Option<DetachedCallViolation> {
                match call {
                    ResolvedCall::StandardLibrary { item, .. } => {
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
                            expression_span: span,
                            function: None,
                            standard_library_name: Some(item.qualified_name),
                        })
                    }
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => self
                        .analysis
                        .function(*function)
                        .requires_attached_process
                        .then_some(DetachedCallViolation {
                            expression_span: span,
                            function: Some(*function),
                            standard_library_name: None,
                        }),
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                }
            }
        }
        impl TypedVisitor for Validator<'_> {
            fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
                let violation = program
                    .call(expression.id)
                    .and_then(|call| self.violation(call, expression.span, program));
                if let Some(violation) = violation {
                    self.violations.push(violation);
                }
                hir::walk_typed_expression(self, expression, program);
            }

            fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
                if let hir::TypedStatementKind::Assign { assignment, .. } = &statement.kind
                    && let Some(call) = &assignment.operator
                    && let Some(violation) = self.violation(call, assignment.span, program)
                {
                    self.violations.push(violation);
                }
                hir::walk_typed_statement(self, statement, program);
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
        Effect::ReadsRuntime => 4,
        Effect::ReadsProcess => 5,
        Effect::RequiresAttachedProcess => 6,
        Effect::Retryable => 7,
        Effect::Suspends => 8,
        Effect::CancelsOnProcessClose => 9,
        Effect::WritesTimer => 10,
        Effect::WritesRuntime => 11,
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

    #[test]
    fn catalog_source_bodies_derive_direct_and_transitive_suspension() {
        let library = crate::stdlib::StandardLibrary::new();
        for item in [
            crate::stdlib::StdlibItemId::CatalogSuspensionProbeWaitOneTick,
            crate::stdlib::StdlibItemId::CatalogSuspensionProbeWaitThroughHelper,
        ] {
            let semantics = library.operation_semantics(item);
            assert_eq!(semantics.suspension, SuspensionKind::Suspends);
            assert_eq!(semantics.availability, Availability::OnAttach);
            assert_eq!(semantics.cancellation, CancellationKind::ProcessClose);
        }

        let awaited = r#"
state "game.exe" {}
onAttach {
    await CatalogSuspensionProbe.waitThroughHelper()
}
"#;
        crate::check(crate::lower(crate::parse(awaited).unwrap())).unwrap();

        let synchronous = r#"
state "game.exe" {}
onAttach {
    CatalogSuspensionProbe.waitThroughHelper()
}
"#;
        let mut database = crate::database::CompilerDatabase::new(synchronous);
        let recovered = database.recovering_check().unwrap();
        assert!(recovered.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == crate::DiagnosticCode::MustUse
                && diagnostic.message == "unused result of `async operation`"
        }));
    }

    #[test]
    fn source_functions_infer_and_validate_async_results() {
        let source = r#"
state "game.exe" {}

fn loadModule() -> async Module {
    let module = await process.module("game.dll")
    return module
}

fn loadModuleIndirectly() {
    let module = await loadModule()
    return module
}

onAttach {
    let module = await loadModuleIndirectly()
    print(module.address)
}
"#;
        let mut database = crate::database::CompilerDatabase::new(source);
        let recovered = database.recovering_check().unwrap();
        assert!(recovered.diagnostics().is_empty());
        for name in ["loadModule", "loadModuleIndirectly"] {
            let function = recovered
                .syntax()
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap();
            assert_eq!(
                recovered
                    .effects()
                    .unwrap()
                    .function(function.id)
                    .suspension,
                SuspensionKind::Suspends
            );
        }

        let hints = crate::inlay_hints::inferred_type_hints(
            &database.semantic_snapshot().unwrap(),
            crate::ast::Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| hint.label == " -> async Module"));

        crate::compile(source).expect("typed source futures should lower to continuation frames");
    }

    #[test]
    fn explicit_function_results_must_agree_with_inferred_asyncness() {
        let missing = r#"
state "game.exe" {}
fn loadModule() -> Module {
    let module = await process.module("game.dll")
    return module
}
"#;
        let errors = crate::check(crate::lower(crate::parse(missing).unwrap())).unwrap_err();
        let error = errors
            .iter()
            .find(|error| error.message.contains("must be marked `async`"))
            .unwrap();
        let fix = error.fixes.first().expect("async mismatch has a fix");
        assert_eq!(fix.edits[0].replacement, "async ");

        let unnecessary = r#"
state "game.exe" {}
fn answer() -> async i32 {
    return 42
}
"#;
        let errors = crate::check(crate::lower(crate::parse(unnecessary).unwrap())).unwrap_err();
        assert!(errors.iter().any(|error| {
            error.message == "function `answer` is declared async but never suspends"
        }));
    }

    #[test]
    fn awaited_values_can_be_returned_directly() {
        let source = r#"
state "game.exe" {}

fn loadUnity() {
    return await Unity.il2cpp(2020)
}
"#;
        let checked = crate::check(crate::lower(crate::parse(source).unwrap())).unwrap();
        let function = &checked.syntax().functions[0];
        assert!(matches!(
            function.body.statements.as_slice(),
            [crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr {
                    kind: crate::ast::ExprKind::Suspend {
                        mode: crate::ast::SuspensionMode::Await,
                        ..
                    },
                    ..
                }),
                ..
            }]
        ));
        assert_eq!(
            checked.effects().function(function.id).suspension,
            SuspensionKind::Suspends
        );
        let result = checked.semantics().function_result(function.id).unwrap();
        let crate::types::TypeKind::Async { value, .. } = checked.semantics().types().kind(result)
        else {
            panic!("an async function should expose an async call result")
        };
        assert!(matches!(
            checked.semantics().types().kind(*value),
            crate::types::TypeKind::Standard(_)
        ));
        assert_eq!(
            checked.semantics().function_completion(function.id),
            Some(*value)
        );
        crate::compile(source)
            .expect("an unused async helper should compile without a parser error");
    }
}
