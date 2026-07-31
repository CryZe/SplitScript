//! Wasm-oriented control-flow and storage plans lowered from typed HIR.
//!
//! This IR deliberately remains close to structured WebAssembly. It owns
//! block terminators, suspension continuations, user/scratch locals, and the
//! complete expression plan consumed by WebAssembly emission. Expression
//! nodes retain semantic IDs and type/conversion edges without depending on
//! source-shaped typed HIR during backend encoding.

use std::collections::HashSet;

mod visit;
pub use visit::{
    Visitor, visit_expression_children, walk_expression, walk_statement, walk_terminator,
};

use crate::{
    ast::{
        ActionKind, BinaryOp, EnumTypeId, ExprId, FunctionId, OptionTypeId, PatternId,
        RecordFieldId, RecordId, ResultTypeId, SuspensionMode, UnaryOp, ValueId,
    },
    hir::{
        self, ExpressionResolution, ImplicitConversion, TypedExpression, TypedExpressionKind,
        TypedFallbackBranch, TypedInterpolatedPart, TypedPattern, TypedProgram, TypedStatementKind,
    },
    intrinsic_registry::{self, ScratchPolicy, ScratchType},
    semantic::{
        ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedValue, ResolvedWrapperPattern,
        SemanticModel, ValueConversion,
    },
    stdlib::{CancellationKind, Implementation, IntrinsicId, StandardLibrary},
    types::{BuiltinType, TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyOwner {
    Function(FunctionId),
    Action(ActionKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(usize);

impl LocalId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AsyncStateId(u32);

impl AsyncStateId {
    pub const ENTRY: Self = Self(0);

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalPurpose {
    Value(ValueId),
    MatchValue(ExprId),
    MatchBinding(PatternId),
    FallbackValue(ExprId),
    IntrinsicScratch { expression: ExprId, slot: u8 },
    SuspensionScratch(ExprId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub id: LocalId,
    pub ty: TypeId,
    pub purpose: LocalPurpose,
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub id: ExprId,
    pub ty: TypeId,
    pub kind: ExpressionKind,
    /// Type-checker-inserted wrapper lift on this expression edge.
    pub conversion: Option<ValueConversion>,
}

#[derive(Debug, Clone)]
pub enum ExpressionKind {
    None,
    Bool(bool),
    Int(u64),
    Float(f64),
    String(String),
    InterpolatedString(Vec<InterpolatedPart>),
    Signature(String),
    Array(Vec<ExprId>),
    Record {
        record: RecordId,
        fields: Vec<(RecordFieldId, ExprId)>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        payload: Option<ExprId>,
    },
    Path {
        root: Option<ResolvedValue>,
        members: Vec<ResolvedMember>,
    },
    Member {
        receiver: ExprId,
        members: Vec<ResolvedMember>,
    },
    Unary {
        op: UnaryOp,
        operand: ExprId,
    },
    Cast {
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        target: CallTarget,
        arguments: Vec<ExprId>,
    },
    If {
        condition: ExprId,
        then_expr: ExprId,
        else_expr: ExprId,
    },
    Fallback {
        value: ExprId,
        fallback: FallbackBranch,
    },
    Propagate {
        value: ExprId,
        target: TypeId,
    },
    Match {
        value: ExprId,
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    UserFunction {
        function: FunctionId,
    },
    UserMethod {
        function: FunctionId,
        receiver: ResolvedValue,
        receiver_type: TypeId,
        receiver_members: Vec<ResolvedMember>,
    },
    Intrinsic {
        item: crate::stdlib::StdlibItemId,
        intrinsic: IntrinsicId,
        type_arguments: Vec<TypeId>,
        receiver: Option<ResolvedValue>,
        receiver_type: Option<TypeId>,
        receiver_members: Vec<ResolvedMember>,
    },
    ResultError {
        result: ResultTypeId,
    },
    OptionSome {
        option: OptionTypeId,
    },
    ResultSuccess {
        result: ResultTypeId,
    },
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Text(String),
    Expression {
        expression: ExprId,
        string_conversion_source: Option<TypeId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackBranch {
    Value(ExprId),
    Return(Option<ExprId>),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern_id: PatternId,
    pub pattern: LoweredPattern,
    pub guard: Option<ExprId>,
    pub value: ExprId,
}

#[derive(Debug, Clone)]
pub enum LoweredPattern {
    Enum {
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        binding: Option<ValueId>,
    },
    Bool(bool),
    Int(u64),
    OptionNone(OptionTypeId),
    OptionSome {
        option: OptionTypeId,
        binding: Option<ValueId>,
    },
    ResultSuccess {
        result: ResultTypeId,
        binding: Option<ValueId>,
    },
    ResultError {
        result: ResultTypeId,
        binding: Option<ValueId>,
    },
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub owner: BodyOwner,
    pub entry: Block,
    pub locals: Vec<Local>,
    /// Source locals retained by at least one poll or continuation state.
    /// Only these values need storage in the async continuation frame.
    pub frame_values: Vec<ValueId>,
    /// Structured lifetime that owns every cancellable suspension in this body.
    pub cancellation_region: Option<CancellationRegion>,
    /// Entry, poll, and continuation states in this async body.
    pub async_state_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationRegion {
    ProcessLifetime,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Store {
        target: ValueId,
        /// Whether this store introduces the body's local rather than assigning
        /// an existing local/global. Local planning consumes this semantic
        /// distinction instead of re-reading typed HIR.
        declaration: bool,
        op: Option<BinaryOp>,
        value: ExprId,
    },
    Evaluate {
        expression: ExprId,
        discard_result: bool,
    },
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    While {
        condition: ExprId,
        body: Block,
    },
}

#[derive(Debug, Clone, Default)]
pub enum Terminator {
    #[default]
    Fallthrough,
    Break,
    Continue,
    AsyncWhile {
        condition: ExprId,
        body: Box<Block>,
        continuation: Box<Block>,
        header_state: AsyncStateId,
        exit_state: AsyncStateId,
    },
    Return(Option<ExprId>),
    Throw {
        error: ExprId,
        target: TypeId,
    },
    Suspend {
        mode: SuspensionMode,
        binding: Option<ValueId>,
        value: ExprId,
        /// State retried while the awaited operation remains pending.
        poll_state: AsyncStateId,
        /// State entered after the awaited operation succeeds.
        resume_state: AsyncStateId,
        /// Region whose cancellation discards this suspended computation.
        cancellation: Option<CancellationRegion>,
        /// Source locals needed to retry this poll or execute its continuation.
        /// Kept in declaration order so lowering output remains deterministic.
        live_values: Vec<ValueId>,
        continuation: Box<Block>,
    },
}

#[derive(Debug, Clone)]
pub struct StateExpression {
    pub field: ValueId,
    pub expression: ExprId,
    pub locals: Vec<Local>,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    standard_library: StandardLibrary,
    profile: crate::BuildProfile,
    bodies: Vec<Body>,
    global_initializers: Vec<(ValueId, ExprId)>,
    state_expressions: Vec<StateExpression>,
    expressions: Vec<Expression>,
}

impl Program {
    pub(crate) fn lower(
        typed_hir: &TypedProgram,
        semantics: &SemanticModel,
        profile: crate::BuildProfile,
    ) -> Self {
        let expressions = typed_hir
            .expressions()
            .map(|expression| lower_expression(expression, typed_hir.standard_library()))
            .collect::<Vec<_>>();
        let global_initializers = typed_hir
            .global_initializers()
            .filter(|initializer| !initializer.debug_only || profile == crate::BuildProfile::Debug)
            .map(|initializer| (initializer.value, initializer.expression))
            .collect();
        let mut program = Self {
            standard_library: typed_hir.standard_library().clone(),
            profile,
            bodies: Vec::new(),
            global_initializers,
            state_expressions: Vec::new(),
            expressions,
        };
        for function in typed_hir.function_bodies() {
            if function.debug_only && profile == crate::BuildProfile::Release {
                continue;
            }
            let body = lower_body(
                BodyOwner::Function(function.function),
                &function.body,
                typed_hir,
                semantics,
                profile,
                &program,
            );
            program.bodies.push(body);
        }
        for action in typed_hir.action_bodies() {
            let body = lower_body(
                BodyOwner::Action(action.action),
                &action.body,
                typed_hir,
                semantics,
                profile,
                &program,
            );
            program.bodies.push(body);
        }
        program.state_expressions = typed_hir
            .state_sources()
            .map(|(field, expression)| StateExpression {
                field,
                expression,
                locals: plan_expression(expression, &program, semantics),
            })
            .collect();
        program
    }

    pub fn profile(&self) -> crate::BuildProfile {
        self.profile
    }

    pub fn standard_library(&self) -> &StandardLibrary {
        &self.standard_library
    }

    pub fn global_initializers(&self) -> impl Iterator<Item = (ValueId, ExprId)> + '_ {
        self.global_initializers.iter().copied()
    }

    pub fn contains_global(&self, value: ValueId) -> bool {
        self.global_initializers
            .iter()
            .any(|(candidate, _)| *candidate == value)
    }

    pub fn bodies(&self) -> impl Iterator<Item = &Body> {
        self.bodies.iter()
    }

    pub fn body(&self, owner: BodyOwner) -> Option<&Body> {
        self.bodies.iter().find(|body| body.owner == owner)
    }

    pub fn state_expressions(&self) -> impl Iterator<Item = &StateExpression> {
        self.state_expressions.iter()
    }

    pub fn state_expression(&self, field: ValueId) -> Option<&StateExpression> {
        self.state_expressions
            .iter()
            .find(|expression| expression.field == field)
    }

    pub fn expression(&self, id: ExprId) -> Option<&Expression> {
        self.expressions
            .binary_search_by_key(&id.index(), |expression| expression.id.index())
            .ok()
            .map(|index| &self.expressions[index])
    }

    pub fn expressions(&self) -> impl ExactSizeIterator<Item = &Expression> {
        self.expressions.iter()
    }
}

fn lower_expression(expression: &TypedExpression, library: &StandardLibrary) -> Expression {
    let kind = match &expression.kind {
        TypedExpressionKind::None => ExpressionKind::None,
        TypedExpressionKind::Bool(value) => ExpressionKind::Bool(*value),
        TypedExpressionKind::Int { value, .. } => ExpressionKind::Int(*value),
        TypedExpressionKind::Float(value) => ExpressionKind::Float(*value),
        TypedExpressionKind::String(value) => ExpressionKind::String(value.clone()),
        TypedExpressionKind::InterpolatedString(parts) => ExpressionKind::InterpolatedString(
            parts
                .iter()
                .map(|part| match part {
                    TypedInterpolatedPart::Text(value) => InterpolatedPart::Text(value.clone()),
                    TypedInterpolatedPart::Expression {
                        expression,
                        conversion,
                    } => InterpolatedPart::Expression {
                        expression: *expression,
                        string_conversion_source: conversion.map(|conversion| match conversion {
                            ImplicitConversion::ToString { source } => source,
                        }),
                    },
                })
                .collect(),
        ),
        TypedExpressionKind::Signature(value) => ExpressionKind::Signature(value.clone()),
        TypedExpressionKind::Array(elements) => ExpressionKind::Array(elements.clone()),
        TypedExpressionKind::Record { record, fields } => {
            let Some(ExpressionResolution::RecordLiteral {
                fields: resolved_fields,
            }) = &expression.resolution
            else {
                unreachable!("checked record literals have resolved field IDs")
            };
            debug_assert_eq!(fields.len(), resolved_fields.len());
            ExpressionKind::Record {
                record: *record,
                fields: resolved_fields
                    .iter()
                    .copied()
                    .zip(fields.iter().map(|(_, value)| *value))
                    .collect(),
            }
        }
        TypedExpressionKind::Enum {
            enumeration,
            payload,
            ..
        } => {
            let Some(ExpressionResolution::EnumConstructor { variant }) = &expression.resolution
            else {
                unreachable!("checked enum constructors have resolved variant IDs")
            };
            ExpressionKind::Enum {
                enumeration: *enumeration,
                variant: *variant,
                payload: *payload,
            }
        }
        TypedExpressionKind::Path(_) => {
            let Some(ExpressionResolution::ValuePath { root, members }) = &expression.resolution
            else {
                unreachable!("checked value paths have a typed-HIR resolution")
            };
            ExpressionKind::Path {
                root: *root,
                members: members.clone(),
            }
        }
        TypedExpressionKind::Member { receiver, .. } => {
            let Some(ExpressionResolution::Member { members }) = &expression.resolution else {
                unreachable!("checked member expressions have a typed-HIR resolution")
            };
            ExpressionKind::Member {
                receiver: *receiver,
                members: members.clone(),
            }
        }
        TypedExpressionKind::Unary {
            op,
            expression: operand,
        } => ExpressionKind::Unary {
            op: *op,
            operand: *operand,
        },
        TypedExpressionKind::Cast {
            expression: value, ..
        } => ExpressionKind::Cast { value: *value },
        TypedExpressionKind::Binary { op, left, right } => ExpressionKind::Binary {
            op: *op,
            left: *left,
            right: *right,
        },
        TypedExpressionKind::Call { arguments, .. } => {
            let Some(ExpressionResolution::Call(target)) = &expression.resolution else {
                unreachable!("checked calls have a resolved target")
            };
            ExpressionKind::Call {
                target: lower_call_target(target, library),
                arguments: arguments.clone(),
            }
        }
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => ExpressionKind::If {
            condition: *condition,
            then_expr: *then_expr,
            else_expr: *else_expr,
        },
        TypedExpressionKind::Fallback { value, fallback } => ExpressionKind::Fallback {
            value: *value,
            fallback: match fallback {
                TypedFallbackBranch::Value(value) => FallbackBranch::Value(*value),
                TypedFallbackBranch::Return(value) => FallbackBranch::Return(*value),
                TypedFallbackBranch::Break => FallbackBranch::Break,
                TypedFallbackBranch::Continue => FallbackBranch::Continue,
            },
        },
        TypedExpressionKind::Propagate { value, target } => ExpressionKind::Propagate {
            value: *value,
            target: *target,
        },
        TypedExpressionKind::Match { value, arms } => ExpressionKind::Match {
            value: *value,
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern_id: arm.resolution.id,
                    pattern: match &arm.pattern {
                        TypedPattern::Enum {
                            enumeration,
                            binding,
                            ..
                        } => LoweredPattern::Enum {
                            enumeration: *enumeration,
                            variant: arm
                                .resolution
                                .variant
                                .expect("checked enum patterns have resolved variants"),
                            binding: binding.as_ref().map(|binding| binding.id),
                        },
                        TypedPattern::Bool(value) => LoweredPattern::Bool(*value),
                        TypedPattern::Int { value, .. } => LoweredPattern::Int(*value),
                        TypedPattern::None => {
                            let Some(ResolvedWrapperPattern::OptionNone(option)) =
                                arm.resolution.wrapper
                            else {
                                unreachable!("checked None patterns resolve to Options")
                            };
                            LoweredPattern::OptionNone(option)
                        }
                        TypedPattern::OptionSome(binding) => {
                            let Some(ResolvedWrapperPattern::OptionSome(option)) =
                                arm.resolution.wrapper
                            else {
                                unreachable!("checked Some patterns resolve to Options")
                            };
                            LoweredPattern::OptionSome {
                                option,
                                binding: binding.as_ref().map(|binding| binding.id),
                            }
                        }
                        TypedPattern::ResultSuccess(binding) => {
                            let Some(ResolvedWrapperPattern::ResultSuccess(result)) =
                                arm.resolution.wrapper
                            else {
                                unreachable!("checked Ok patterns resolve to Results")
                            };
                            LoweredPattern::ResultSuccess {
                                result,
                                binding: binding.as_ref().map(|binding| binding.id),
                            }
                        }
                        TypedPattern::ResultError(binding) => {
                            let Some(ResolvedWrapperPattern::ResultError(result)) =
                                arm.resolution.wrapper
                            else {
                                unreachable!("checked Err patterns resolve to Results")
                            };
                            LoweredPattern::ResultError {
                                result,
                                binding: binding.as_ref().map(|binding| binding.id),
                            }
                        }
                        TypedPattern::Wildcard => LoweredPattern::Wildcard,
                    },
                    guard: arm.guard,
                    value: arm.value,
                })
                .collect(),
        },
    };
    Expression {
        id: expression.id,
        ty: expression.ty,
        kind,
        conversion: expression.conversion,
    }
}

fn lower_call_target(target: &ResolvedCall, library: &StandardLibrary) -> CallTarget {
    match target {
        ResolvedCall::UserFunction { function } => CallTarget::UserFunction {
            function: *function,
        },
        ResolvedCall::UserMethod {
            function,
            receiver,
            receiver_type,
            receiver_members,
        } => CallTarget::UserMethod {
            function: *function,
            receiver: *receiver,
            receiver_type: *receiver_type,
            receiver_members: receiver_members.clone(),
        },
        ResolvedCall::StandardLibrary {
            item,
            type_arguments,
            receiver,
            receiver_type,
            receiver_members,
        } => {
            let Implementation::Intrinsic(intrinsic) = library.item(*item).implementation;
            CallTarget::Intrinsic {
                item: *item,
                intrinsic,
                type_arguments: type_arguments.clone(),
                receiver: *receiver,
                receiver_type: *receiver_type,
                receiver_members: receiver_members.clone(),
            }
        }
        ResolvedCall::ResultError { result } => CallTarget::ResultError { result: *result },
        ResolvedCall::OptionSome { option } => CallTarget::OptionSome { option: *option },
        ResolvedCall::ResultSuccess { result } => CallTarget::ResultSuccess { result: *result },
    }
}

fn lower_body(
    owner: BodyOwner,
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
    wasm_ir: &Program,
) -> Body {
    let mut entry = if owner == BodyOwner::Action(ActionKind::OnAttach) {
        lower_async_block(block, typed_hir, semantics, profile)
    } else {
        lower_block(block, typed_hir, semantics, profile)
    };
    let mut next_async_state = 1;
    assign_async_states(&mut entry, &mut next_async_state);
    let locals = plan_block(&entry, wasm_ir, semantics);
    let frame_values = plan_frame_values(&mut entry, &locals, wasm_ir);
    let cancellation_region = (owner == BodyOwner::Action(ActionKind::OnAttach))
        .then_some(CancellationRegion::ProcessLifetime);
    Body {
        owner,
        entry,
        locals,
        frame_values,
        cancellation_region,
        async_state_count: next_async_state,
    }
}

fn plan_frame_values(entry: &mut Block, locals: &[Local], program: &Program) -> Vec<ValueId> {
    let local_values = locals
        .iter()
        .filter_map(|local| match local.purpose {
            LocalPurpose::Value(value) => Some(value),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut frame_values = HashSet::new();
    analyze_suspension_liveness(
        entry,
        HashSet::new(),
        &local_values,
        locals,
        program,
        &mut frame_values,
    );
    locals
        .iter()
        .filter_map(|local| match local.purpose {
            LocalPurpose::Value(value) if frame_values.contains(&value) => Some(value),
            _ => None,
        })
        .collect()
}

fn analyze_suspension_liveness(
    block: &mut Block,
    live_after: HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    ordered_locals: &[Local],
    program: &Program,
    frame_values: &mut HashSet<ValueId>,
) -> HashSet<ValueId> {
    let mut live = match &mut block.terminator {
        Terminator::Suspend {
            binding,
            value,
            live_values,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let mut suspension_live = continuation_live.clone();
            collect_expression_values(*value, &mut suspension_live, local_values, program);
            live_values.clear();
            live_values.extend(
                ordered_locals
                    .iter()
                    .filter_map(|local| match local.purpose {
                        LocalPurpose::Value(value) if suspension_live.contains(&value) => {
                            Some(value)
                        }
                        _ => None,
                    }),
            );
            frame_values.extend(live_values.iter().copied());

            let mut before_suspend = continuation_live;
            if let Some(binding) = binding {
                before_suspend.remove(binding);
            }
            collect_expression_values(*value, &mut before_suspend, local_values, program);
            before_suspend
        }
        Terminator::Return(value) => {
            let mut live = HashSet::new();
            if let Some(value) = value {
                collect_expression_values(*value, &mut live, local_values, program);
            }
            live
        }
        Terminator::Break | Terminator::Continue => live_after,
        Terminator::AsyncWhile {
            condition,
            body,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let mut loop_live = continuation_live;
            collect_expression_values(*condition, &mut loop_live, local_values, program);
            loop {
                let body_live = analyze_suspension_liveness(
                    body,
                    loop_live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                let previous_len = loop_live.len();
                loop_live.extend(body_live);
                if loop_live.len() == previous_len {
                    break;
                }
            }
            loop_live
        }
        Terminator::Throw { error, .. } => {
            let mut live = HashSet::new();
            collect_expression_values(*error, &mut live, local_values, program);
            live
        }
        Terminator::Fallthrough => live_after,
    };
    analyze_statements_liveness(
        &mut block.statements,
        &mut live,
        local_values,
        ordered_locals,
        program,
        frame_values,
    );
    live
}

fn analyze_statements_liveness(
    statements: &mut [Statement],
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    ordered_locals: &[Local],
    program: &Program,
    frame_values: &mut HashSet<ValueId>,
) {
    for statement in statements.iter_mut().rev() {
        match statement {
            Statement::Store {
                target, op, value, ..
            } => {
                live.remove(target);
                if op.is_some() && local_values.contains(target) {
                    live.insert(*target);
                }
                collect_expression_values(*value, live, local_values, program);
            }
            Statement::Evaluate { expression, .. } => {
                collect_expression_values(*expression, live, local_values, program);
            }
            Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut then_live = analyze_suspension_liveness(
                    then_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                let else_live = analyze_suspension_liveness(
                    else_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                then_live.extend(else_live);
                collect_expression_values(*condition, &mut then_live, local_values, program);
                *live = then_live;
            }
            Statement::While { condition, body } => {
                let mut body_live = analyze_suspension_liveness(
                    body,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                body_live.extend(live.iter().copied());
                collect_expression_values(*condition, &mut body_live, local_values, program);
                *live = body_live;
            }
        }
    }
}

fn collect_expression_values(
    expression: ExprId,
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    program: &Program,
) {
    struct Collector<'a> {
        live: &'a mut HashSet<ValueId>,
        local_values: &'a HashSet<ValueId>,
    }

    impl Visitor for Collector<'_> {
        fn visit_expression(&mut self, expression: &Expression, program: &Program) {
            let root = match &expression.kind {
                ExpressionKind::Path { root, .. } => *root,
                ExpressionKind::Call {
                    target:
                        CallTarget::UserMethod { receiver, .. }
                        | CallTarget::Intrinsic {
                            receiver: Some(receiver),
                            ..
                        },
                    ..
                } => Some(*receiver),
                _ => None,
            };
            if let Some(ResolvedValue::Variable(value)) = root
                && self.local_values.contains(&value)
            {
                self.live.insert(value);
            }
            walk_expression(self, expression, program);
        }
    }

    Collector { live, local_values }.visit_expression_id(expression, program);
}

fn lower_block(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Block {
    lower_statements(&block.statements, typed_hir, semantics, profile)
}

fn lower_async_block(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Block {
    lower_async_statements(
        &block.statements,
        Block::default(),
        typed_hir,
        semantics,
        profile,
    )
}

fn lower_async_statements(
    statements: &[hir::TypedStatement],
    tail: Block,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Block {
    let mut result = tail;
    for statement in statements.iter().rev() {
        if statement.debug_only && profile == crate::BuildProfile::Release {
            continue;
        }
        match &statement.kind {
            TypedStatementKind::Variable { value, initializer } => {
                result.statements.insert(
                    0,
                    Statement::Store {
                        target: *value,
                        declaration: true,
                        op: None,
                        value: *initializer,
                    },
                );
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                result.statements.insert(
                    0,
                    Statement::Store {
                        target: assignment.target,
                        declaration: false,
                        op: *op,
                        value: *value,
                    },
                );
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } if typed_block_contains_await(then_block, profile)
                || else_block
                    .as_ref()
                    .is_some_and(|block| typed_block_contains_await(block, profile)) =>
            {
                let then_block = lower_async_statements(
                    &then_block.statements,
                    result.clone(),
                    typed_hir,
                    semantics,
                    profile,
                );
                let else_block = else_block.as_ref().map_or_else(
                    || result.clone(),
                    |block| {
                        lower_async_statements(
                            &block.statements,
                            result.clone(),
                            typed_hir,
                            semantics,
                            profile,
                        )
                    },
                );
                result = Block {
                    statements: vec![Statement::If {
                        condition: *condition,
                        then_block,
                        else_block,
                    }],
                    terminator: Terminator::Fallthrough,
                };
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                result.statements.insert(
                    0,
                    Statement::If {
                        condition: *condition,
                        then_block: lower_block(then_block, typed_hir, semantics, profile),
                        else_block: else_block.as_ref().map_or_else(Block::default, |block| {
                            lower_block(block, typed_hir, semantics, profile)
                        }),
                    },
                );
            }
            TypedStatementKind::While { condition, body } => {
                if typed_block_contains_await(body, profile) {
                    let body = lower_async_statements(
                        &body.statements,
                        Block {
                            statements: Vec::new(),
                            terminator: Terminator::Continue,
                        },
                        typed_hir,
                        semantics,
                        profile,
                    );
                    result = Block {
                        statements: Vec::new(),
                        terminator: Terminator::AsyncWhile {
                            condition: *condition,
                            body: Box::new(body),
                            continuation: Box::new(result),
                            header_state: AsyncStateId::ENTRY,
                            exit_state: AsyncStateId::ENTRY,
                        },
                    };
                    continue;
                }
                result.statements.insert(
                    0,
                    Statement::While {
                        condition: *condition,
                        body: lower_block(body, typed_hir, semantics, profile),
                    },
                );
            }
            TypedStatementKind::Break => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Break,
                };
            }
            TypedStatementKind::Continue => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Continue,
                };
            }
            TypedStatementKind::Return(value) => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Return(*value),
                };
            }
            TypedStatementKind::Throw { error, target } => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Throw {
                        error: *error,
                        target: *target,
                    },
                };
            }
            TypedStatementKind::Suspend {
                mode,
                binding,
                value,
            } => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Suspend {
                        mode: *mode,
                        binding: *binding,
                        value: *value,
                        poll_state: AsyncStateId::ENTRY,
                        resume_state: AsyncStateId::ENTRY,
                        cancellation: suspension_cancellation(*mode, *value, typed_hir),
                        live_values: Vec::new(),
                        continuation: Box::new(result),
                    },
                };
            }
            TypedStatementKind::Expression(expression) => {
                let ty = typed_hir
                    .expression(*expression)
                    .expect("statement expression belongs to typed HIR")
                    .ty;
                result.statements.insert(
                    0,
                    Statement::Evaluate {
                        expression: *expression,
                        discard_result: !matches!(
                            semantics.types().kind(ty),
                            TypeKind::Builtin(BuiltinType::Void)
                        ),
                    },
                );
            }
        }
    }
    result
}

fn typed_block_contains_await(block: &hir::TypedBlock, profile: crate::BuildProfile) -> bool {
    block.statements.iter().any(|statement| {
        if statement.debug_only && profile == crate::BuildProfile::Release {
            return false;
        }
        match &statement.kind {
            TypedStatementKind::Suspend { .. } => true,
            TypedStatementKind::If {
                then_block,
                else_block,
                ..
            } => {
                typed_block_contains_await(then_block, profile)
                    || else_block
                        .as_ref()
                        .is_some_and(|block| typed_block_contains_await(block, profile))
            }
            TypedStatementKind::While { body, .. } => typed_block_contains_await(body, profile),
            _ => false,
        }
    })
}

fn assign_async_states(block: &mut Block, next: &mut u32) {
    for statement in &mut block.statements {
        match statement {
            Statement::If {
                then_block,
                else_block,
                ..
            } => {
                assign_async_states(then_block, next);
                assign_async_states(else_block, next);
            }
            Statement::While { body, .. } => assign_async_states(body, next),
            Statement::Store { .. } | Statement::Evaluate { .. } => {}
        }
    }
    if let Terminator::Suspend {
        poll_state,
        resume_state,
        continuation,
        ..
    } = &mut block.terminator
    {
        *poll_state = AsyncStateId(*next);
        *next += 1;
        *resume_state = AsyncStateId(*next);
        *next += 1;
        assign_async_states(continuation, next);
    } else if let Terminator::AsyncWhile {
        body,
        continuation,
        header_state,
        exit_state,
        ..
    } = &mut block.terminator
    {
        *header_state = AsyncStateId(*next);
        *next += 1;
        *exit_state = AsyncStateId(*next);
        *next += 1;
        assign_async_states(body, next);
        assign_async_states(continuation, next);
    }
}

fn lower_statements(
    statements: &[hir::TypedStatement],
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Block {
    let mut block = Block::default();
    for (index, statement) in statements.iter().enumerate() {
        if statement.debug_only && profile == crate::BuildProfile::Release {
            continue;
        }
        match &statement.kind {
            TypedStatementKind::Variable { value, initializer } => {
                block.statements.push(Statement::Store {
                    target: *value,
                    declaration: true,
                    op: None,
                    value: *initializer,
                });
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                block.statements.push(Statement::Store {
                    target: assignment.target,
                    declaration: false,
                    op: *op,
                    value: *value,
                });
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => block.statements.push(Statement::If {
                condition: *condition,
                then_block: lower_block(then_block, typed_hir, semantics, profile),
                else_block: else_block.as_ref().map_or_else(Block::default, |block| {
                    lower_block(block, typed_hir, semantics, profile)
                }),
            }),
            TypedStatementKind::While { condition, body } => {
                block.statements.push(Statement::While {
                    condition: *condition,
                    body: lower_block(body, typed_hir, semantics, profile),
                });
            }
            TypedStatementKind::Break => {
                block.terminator = Terminator::Break;
                return block;
            }
            TypedStatementKind::Continue => {
                block.terminator = Terminator::Continue;
                return block;
            }
            TypedStatementKind::Return(value) => {
                block.terminator = Terminator::Return(*value);
                return block;
            }
            TypedStatementKind::Throw { error, target } => {
                block.terminator = Terminator::Throw {
                    error: *error,
                    target: *target,
                };
                return block;
            }
            TypedStatementKind::Suspend {
                mode,
                binding,
                value,
            } => {
                block.terminator = Terminator::Suspend {
                    mode: *mode,
                    binding: *binding,
                    value: *value,
                    poll_state: AsyncStateId::ENTRY,
                    resume_state: AsyncStateId::ENTRY,
                    cancellation: suspension_cancellation(*mode, *value, typed_hir),
                    live_values: Vec::new(),
                    continuation: Box::new(lower_statements(
                        &statements[index + 1..],
                        typed_hir,
                        semantics,
                        profile,
                    )),
                };
                return block;
            }
            TypedStatementKind::Expression(expression) => {
                let ty = typed_hir
                    .expression(*expression)
                    .expect("statement expression belongs to typed HIR")
                    .ty;
                block.statements.push(Statement::Evaluate {
                    expression: *expression,
                    discard_result: !matches!(
                        semantics.types().kind(ty),
                        TypeKind::Builtin(BuiltinType::Void)
                    ),
                });
            }
        }
    }
    block
}

fn suspension_cancellation(
    mode: SuspensionMode,
    expression: ExprId,
    typed_hir: &TypedProgram,
) -> Option<CancellationRegion> {
    if mode == SuspensionMode::Retry {
        return Some(CancellationRegion::ProcessLifetime);
    }
    let ResolvedCall::StandardLibrary { item, .. } = typed_hir.call(expression)? else {
        return None;
    };
    (typed_hir
        .standard_library()
        .item(*item)
        .operation_semantics()
        .cancellation
        == CancellationKind::ProcessClose)
        .then_some(CancellationRegion::ProcessLifetime)
}

fn plan_block(block: &Block, program: &Program, semantics: &SemanticModel) -> Vec<Local> {
    let mut planner = LocalPlanner::new(semantics);
    Visitor::visit_block(&mut planner, block, program);
    planner.locals
}

fn plan_expression(expression: ExprId, program: &Program, semantics: &SemanticModel) -> Vec<Local> {
    let mut planner = LocalPlanner::new(semantics);
    planner.visit_expression_id(expression, program);
    planner.locals
}

struct LocalPlanner<'a> {
    semantics: &'a SemanticModel,
    locals: Vec<Local>,
}

impl<'a> LocalPlanner<'a> {
    fn new(semantics: &'a SemanticModel) -> Self {
        Self {
            semantics,
            locals: Vec::new(),
        }
    }

    fn push(&mut self, ty: TypeId, purpose: LocalPurpose) {
        let id = LocalId(self.locals.len());
        self.locals.push(Local { id, ty, purpose });
    }

    fn value(&mut self, value: ValueId) {
        let ty = self
            .semantics
            .value_type(value)
            .expect("checked local values have semantic types");
        self.push(ty, LocalPurpose::Value(value));
    }

    fn push_intrinsic_scratch(
        &mut self,
        expression: ExprId,
        expression_ty: TypeId,
        policy: ScratchPolicy,
    ) {
        let ty = match policy.ty {
            ScratchType::Core(core) => self.semantics.types().id_for_core(core),
            ScratchType::Expression => expression_ty,
            ScratchType::ResultValue => {
                let TypeKind::Result { value, .. } = self.semantics.types().kind(expression_ty)
                else {
                    unreachable!("result-value scratch requires a Result expression")
                };
                *value
            }
        };
        for slot in 0..policy.slots {
            self.push(ty, LocalPurpose::IntrinsicScratch { expression, slot });
        }
    }
}

impl Visitor for LocalPlanner<'_> {
    fn visit_statement(&mut self, statement: &Statement, program: &Program) {
        if let Statement::Store {
            target,
            declaration: true,
            ..
        } = statement
        {
            self.value(*target);
        }
        walk_statement(self, statement, program);
    }

    fn visit_terminator(&mut self, terminator: &Terminator, program: &Program) {
        if let Terminator::Suspend {
            mode,
            binding,
            value,
            ..
        } = terminator
        {
            if let Some(binding) = binding {
                self.value(*binding);
            }
            let result_type = program
                .expression(*value)
                .expect("suspended expression belongs to Wasm IR")
                .ty;
            if *mode == SuspensionMode::Retry {
                self.push(result_type, LocalPurpose::SuspensionScratch(*value));
            } else if let Some(intrinsic) = resolved_intrinsic(program, *value)
                && let Some(policy) = intrinsic_registry::contract(intrinsic).async_scratch
            {
                self.push_intrinsic_scratch(*value, result_type, policy);
            }
        }
        walk_terminator(self, terminator, program);
    }

    fn visit_expression(&mut self, expression: &Expression, program: &Program) {
        if let ExpressionKind::Match { value, arms } = &expression.kind {
            self.visit_expression_id(*value, program);
            let value_type = program
                .expression(*value)
                .expect("match input belongs to Wasm IR")
                .ty;
            self.push(value_type, LocalPurpose::MatchValue(expression.id));
            for arm in arms {
                let binding = match &arm.pattern {
                    LoweredPattern::Enum {
                        binding: Some(binding),
                        ..
                    }
                    | LoweredPattern::OptionSome {
                        binding: Some(binding),
                        ..
                    }
                    | LoweredPattern::ResultSuccess {
                        binding: Some(binding),
                        ..
                    }
                    | LoweredPattern::ResultError {
                        binding: Some(binding),
                        ..
                    } => Some(*binding),
                    _ => None,
                };
                if let Some(binding) = binding {
                    let binding_type = self
                        .semantics
                        .value_type(binding)
                        .expect("checked pattern bindings have types");
                    self.push(binding_type, LocalPurpose::MatchBinding(arm.pattern_id));
                }
                if let Some(guard) = arm.guard {
                    self.visit_expression_id(guard, program);
                }
                self.visit_expression_id(arm.value, program);
            }
            return;
        }

        if let ExpressionKind::Fallback { value, fallback } = &expression.kind {
            self.visit_expression_id(*value, program);
            let value_type = program
                .expression(*value)
                .expect("fallback input belongs to Wasm IR")
                .ty;
            self.push(value_type, LocalPurpose::FallbackValue(expression.id));
            match fallback {
                FallbackBranch::Value(fallback) => {
                    self.visit_expression_id(*fallback, program);
                }
                FallbackBranch::Return(Some(value)) => {
                    self.visit_expression_id(*value, program);
                }
                FallbackBranch::Return(None) | FallbackBranch::Break | FallbackBranch::Continue => {
                }
            }
            return;
        }

        if let ExpressionKind::Propagate { value, .. } = expression.kind {
            self.visit_expression_id(value, program);
            let input_type = program
                .expression(value)
                .expect("propagated input belongs to Wasm IR")
                .ty;
            self.push(input_type, LocalPurpose::FallbackValue(expression.id));
            return;
        }

        walk_expression(self, expression, program);
        if let ExpressionKind::Call {
            target: CallTarget::Intrinsic { intrinsic, .. },
            ..
        } = expression.kind
            && let Some(policy) = intrinsic_registry::contract(intrinsic).synchronous_scratch
        {
            self.push_intrinsic_scratch(expression.id, expression.ty, policy);
        }
    }
}

fn resolved_intrinsic(program: &Program, expression: ExprId) -> Option<IntrinsicId> {
    let ExpressionKind::Call {
        target: CallTarget::Intrinsic { intrinsic, .. },
        ..
    } = &program.expression(expression)?.kind
    else {
        return None;
    };
    Some(*intrinsic)
}
