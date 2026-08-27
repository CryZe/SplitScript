//! Operational-effect inference over calls and first-class values.

mod polymorphic;

use crate::{
    ast::{ActionKind, FunctionId, Span},
    hir::{self, TypedExpression, TypedProgram, TypedVisitor},
    semantic::{ResolvedCall, SemanticModel},
    stdlib::{
        Availability, CancellationKind, Effect, EffectSet, OperationMetadata, SuspensionKind,
    },
};

/// A latent operation a function performs through one of its parameters.
///
/// These requirements are inferred and substituted at call sites; they are
/// not part of SplitScript's source-level type spelling or runtime ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatentParameterOperation {
    pub parameter: usize,
    pub fields: Vec<crate::semantic::ResolvedMember>,
    pub kind: LatentOperationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatentOperationKind {
    Invoke,
    Iterate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionOperationSemantics {
    pub effects: Vec<Effect>,
    /// Parameter-dependent operations that become concrete when a function is
    /// called with a particular callable or iterable value.
    pub latent_parameter_operations: Vec<LatentParameterOperation>,
    pub requires_attached_process: bool,
    pub requires_state_snapshots: bool,
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
    calls: std::collections::HashMap<crate::ast::ExprId, FunctionOperationSemantics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachedProcessViolation {
    pub action: ActionKind,
    pub expression_span: Span,
    pub function: Option<FunctionId>,
    pub standard_library_name: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSnapshotContext {
    Action(ActionKind),
    StateSource,
    StateTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateSnapshotViolation {
    pub context: StateSnapshotContext,
    pub expression_span: Span,
    pub function: Option<FunctionId>,
    pub standard_library_name: Option<&'static str>,
}

/// Whether code in this lifecycle action executes with a selected process
/// provider. Keep this as the shared source of truth for semantic validation
/// and editor candidate filtering as new lifecycle contexts are introduced.
pub const fn action_has_attached_process(action: ActionKind) -> bool {
    match action {
        ActionKind::Setup | ActionKind::OnDetach | ActionKind::OnStart | ActionKind::OnReset => {
            false
        }
        ActionKind::OnAttach
        | ActionKind::OnStateReady
        | ActionKind::WhileAttached
        | ActionKind::Start
        | ActionKind::Split
        | ActionKind::Reset
        | ActionKind::IsLoading
        | ActionKind::GameTime => true,
    }
}

/// Whether a lifecycle action runs after the first complete state snapshot has
/// been committed. A process can close before its first snapshot, so
/// `onDetach` cannot expose stale or default-initialized storage as real state.
pub const fn action_has_state_snapshots(action: ActionKind) -> bool {
    matches!(
        action,
        ActionKind::OnStateReady
            | ActionKind::WhileAttached
            | ActionKind::Start
            | ActionKind::Split
            | ActionKind::Reset
            | ActionKind::IsLoading
            | ActionKind::GameTime
    )
}

fn implicit_display_callees(
    expression: &TypedExpression,
    program: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Vec<FunctionId> {
    let mut functions = Vec::new();
    for source in hir::implicit_display_types(expression, program, semantics) {
        functions.extend(capabilities.display_method_implementations(source, semantics));
    }
    functions.sort_by_key(|function| function.index());
    functions.dedup();
    functions
}

impl OperationAnalysis {
    pub fn infer(
        syntax: &crate::ast::Program,
        program: &TypedProgram,
        semantics: &SemanticModel,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        scoped_globals: &crate::scoped_globals::ScopedGlobalAnalysis,
    ) -> Self {
        polymorphic::infer(syntax, program, semantics, capabilities, scoped_globals)
    }

    pub fn function(&self, function: FunctionId) -> FunctionOperationSemantics {
        self.functions
            .get(function.index())
            .cloned()
            .unwrap_or_default()
    }

    /// Operation semantics after substituting the concrete callable values at
    /// one invocation site.
    pub fn call(&self, expression: crate::ast::ExprId) -> Option<&FunctionOperationSemantics> {
        self.calls.get(&expression)
    }

    pub fn attached_process_violations(
        &self,
        program: &TypedProgram,
        semantics: &SemanticModel,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) -> Vec<AttachedProcessViolation> {
        struct Validator<'a> {
            analysis: &'a OperationAnalysis,
            semantics: &'a SemanticModel,
            capabilities: &'a crate::capabilities::CapabilityAnalysis,
            action: ActionKind,
            violations: Vec<AttachedProcessViolation>,
        }
        impl Validator<'_> {
            fn call_site_violation(
                &self,
                expression: &TypedExpression,
                program: &TypedProgram,
            ) -> Option<AttachedProcessViolation> {
                let operation = self.analysis.call(expression.id)?;
                if !operation.requires_attached_process {
                    return None;
                }
                let (function, standard_library_name) =
                    call_identity(program.call(expression.id), program, "callable");
                Some(AttachedProcessViolation {
                    action: self.action,
                    expression_span: expression.span,
                    function,
                    standard_library_name,
                })
            }

            fn violation(
                &self,
                call: &ResolvedCall,
                span: Span,
                program: &TypedProgram,
            ) -> Option<AttachedProcessViolation> {
                match call {
                    ResolvedCall::StandardLibrary { item, .. } => {
                        let item = program.standard_library().item(*item);
                        let mut functions = program.library_functions(item.id).peekable();
                        let requires_attached_process = if functions.peek().is_some() {
                            functions.any(|function| {
                                self.analysis.function(function).requires_attached_process
                            })
                        } else {
                            program
                                .standard_library()
                                .operation_semantics(item.id)
                                .requires_attached_process
                        };
                        requires_attached_process.then_some(AttachedProcessViolation {
                            action: self.action,
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
                        .then_some(AttachedProcessViolation {
                            action: self.action,
                            expression_span: span,
                            function: Some(*function),
                            standard_library_name: None,
                        }),
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                }
            }
        }
        impl TypedVisitor for Validator<'_> {
            fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
                for function in
                    implicit_display_callees(expression, program, self.semantics, self.capabilities)
                {
                    if self.analysis.function(function).requires_attached_process {
                        self.violations.push(AttachedProcessViolation {
                            action: self.action,
                            expression_span: expression.span,
                            function: Some(function),
                            standard_library_name: None,
                        });
                    }
                }
                let violation = self.call_site_violation(expression, program).or_else(|| {
                    program
                        .call(expression.id)
                        .and_then(|call| self.violation(call, expression.span, program))
                });
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
                if let hir::TypedStatementKind::IndexAssign { assignment, .. } = &statement.kind
                    && let Some(violation) =
                        self.violation(&assignment.operator, assignment.span, program)
                {
                    self.violations.push(violation);
                }
                hir::walk_typed_statement(self, statement, program);
            }
        }

        let mut violations = Vec::new();
        for action in program
            .action_bodies()
            .filter(|action| !action_has_attached_process(action.action))
        {
            let mut validator = Validator {
                analysis: self,
                semantics,
                capabilities,
                action: action.action,
                violations: Vec::new(),
            };
            validator.visit_block(&action.body, program);
            violations.extend(validator.violations);
        }
        violations
    }

    pub fn state_snapshot_violations(
        &self,
        program: &TypedProgram,
        semantics: &SemanticModel,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) -> Vec<StateSnapshotViolation> {
        struct Validator<'a> {
            analysis: &'a OperationAnalysis,
            semantics: &'a SemanticModel,
            capabilities: &'a crate::capabilities::CapabilityAnalysis,
            context: StateSnapshotContext,
            violations: Vec<StateSnapshotViolation>,
        }
        impl Validator<'_> {
            fn call_site_violation(
                &self,
                expression: &TypedExpression,
                program: &TypedProgram,
            ) -> Option<StateSnapshotViolation> {
                let operation = self.analysis.call(expression.id)?;
                if !operation.requires_state_snapshots {
                    return None;
                }
                let (function, standard_library_name) =
                    call_identity(program.call(expression.id), program, "callable");
                Some(StateSnapshotViolation {
                    context: self.context,
                    expression_span: expression.span,
                    function,
                    standard_library_name,
                })
            }

            fn violation(
                &self,
                call: &ResolvedCall,
                span: Span,
                program: &TypedProgram,
            ) -> Option<StateSnapshotViolation> {
                let (requires_state_snapshots, function, standard_library_name) = match call {
                    ResolvedCall::StandardLibrary { item, .. } => {
                        let item = program.standard_library().item(*item);
                        let requires = program.library_functions(item.id).any(|function| {
                            self.analysis.function(function).requires_state_snapshots
                        });
                        (requires, None, Some(item.qualified_name))
                    }
                    ResolvedCall::UserFunction { function, .. }
                    | ResolvedCall::UserMethod { function, .. } => (
                        self.analysis.function(*function).requires_state_snapshots,
                        Some(*function),
                        None,
                    ),
                    ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. } => (false, None, None),
                };
                requires_state_snapshots.then_some(StateSnapshotViolation {
                    context: self.context,
                    expression_span: span,
                    function,
                    standard_library_name,
                })
            }
        }
        impl TypedVisitor for Validator<'_> {
            fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
                for function in
                    implicit_display_callees(expression, program, self.semantics, self.capabilities)
                {
                    if self.analysis.function(function).requires_state_snapshots {
                        self.violations.push(StateSnapshotViolation {
                            context: self.context,
                            expression_span: expression.span,
                            function: Some(function),
                            standard_library_name: None,
                        });
                    }
                }
                if let Some(violation) =
                    self.call_site_violation(expression, program).or_else(|| {
                        program
                            .call(expression.id)
                            .and_then(|call| self.violation(call, expression.span, program))
                    })
                {
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
                if let hir::TypedStatementKind::IndexAssign { assignment, .. } = &statement.kind
                    && let Some(violation) =
                        self.violation(&assignment.operator, assignment.span, program)
                {
                    self.violations.push(violation);
                }
                hir::walk_typed_statement(self, statement, program);
            }
        }

        let mut violations = Vec::new();
        for action in program
            .action_bodies()
            .filter(|action| !action_has_state_snapshots(action.action))
        {
            let mut validator = Validator {
                analysis: self,
                semantics,
                capabilities,
                context: StateSnapshotContext::Action(action.action),
                violations: Vec::new(),
            };
            validator.visit_block(&action.body, program);
            violations.extend(validator.violations);
        }
        for (_, expression) in program.state_sources() {
            let mut validator = Validator {
                analysis: self,
                semantics,
                capabilities,
                context: StateSnapshotContext::StateSource,
                violations: Vec::new(),
            };
            if let Some(expression) = program.expression(expression) {
                validator.visit_expression(expression, program);
            }
            violations.extend(validator.violations);
        }
        for transform in program.state_transforms() {
            let mut validator = Validator {
                analysis: self,
                semantics,
                capabilities,
                context: StateSnapshotContext::StateTransform,
                violations: Vec::new(),
            };
            if let Some(expression) = program.expression(transform.expression) {
                validator.visit_expression(expression, program);
            }
            violations.extend(validator.violations);
        }
        violations
    }
}

fn call_identity(
    call: Option<&ResolvedCall>,
    program: &TypedProgram,
    dynamic_name: &'static str,
) -> (Option<FunctionId>, Option<&'static str>) {
    match call {
        Some(ResolvedCall::UserFunction { function, .. })
        | Some(ResolvedCall::UserMethod { function, .. }) => (Some(*function), None),
        Some(ResolvedCall::StandardLibrary { item, .. }) => (
            None,
            Some(program.standard_library().item(*item).qualified_name),
        ),
        Some(
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. },
        ) => (None, None),
        None => (None, Some(dynamic_name)),
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
        latent_parameter_operations: Vec::new(),
        requires_attached_process: operation.requires_attached_process,
        requires_state_snapshots: operation.requires_state_snapshots,
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
        Effect::RequiresStateSnapshots => 7,
        Effect::WritesCurrentState => 8,
        Effect::Suspends => 9,
        Effect::CancelsOnProcessClose => 10,
        Effect::WritesTimer => 11,
        Effect::WritesRuntime => 12,
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
            [crate::ast::Stmt::Expression(crate::ast::Expr {
                kind: crate::ast::ExprKind::Return(Some(value)),
                ..
            })] if matches!(
                value.kind,
                crate::ast::ExprKind::Suspend {
                    mode: crate::ast::SuspensionMode::Await,
                    ..
                }
            )
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
