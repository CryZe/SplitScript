//! Wasm-oriented control-flow and storage plans lowered from typed HIR.
//!
//! This IR deliberately remains close to structured WebAssembly. It owns
//! block terminators, suspension continuations, user/scratch locals, and the
//! complete expression plan consumed by WebAssembly emission. Expression
//! nodes retain semantic IDs and type/conversion edges without depending on
//! source-shaped typed HIR during backend encoding.

use std::collections::HashSet;

use crate::{
    ast::{
        ActionKind, BinaryOp, EnumId, EnumVariantId, ExprId, FunctionId, MatchPattern,
        OptionTypeId, PatternId, RecordFieldId, RecordId, ResultTypeId, SuspensionMode, UnaryOp,
        ValueId,
    },
    hir::{
        self, ExpressionResolution, ImplicitConversion, TypedExpression, TypedExpressionKind,
        TypedFallbackBranch, TypedInterpolatedPart, TypedProgram, TypedStatementKind, TypedVisitor,
    },
    semantic::{
        ResolvedCall, ResolvedMember, ResolvedValue, ResolvedWrapperPattern, SemanticModel,
        ValueConversion,
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
        enumeration: EnumId,
        variant: EnumVariantId,
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
        target: ResolvedCall,
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
        enumeration: EnumId,
        variant: EnumVariantId,
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
        let mut bodies = Vec::new();
        for function in typed_hir.function_bodies() {
            if function.debug_only && profile == crate::BuildProfile::Release {
                continue;
            }
            bodies.push(lower_body(
                BodyOwner::Function(function.function),
                &function.body,
                typed_hir,
                semantics,
                profile,
            ));
        }
        for action in typed_hir.action_bodies() {
            bodies.push(lower_body(
                BodyOwner::Action(action.action),
                &action.body,
                typed_hir,
                semantics,
                profile,
            ));
        }
        let global_initializers = typed_hir
            .global_initializers()
            .filter(|initializer| !initializer.debug_only || profile == crate::BuildProfile::Debug)
            .map(|initializer| (initializer.value, initializer.expression))
            .collect();
        let state_expressions = typed_hir
            .state_sources()
            .map(|(field, expression)| StateExpression {
                field,
                expression,
                locals: plan_expression(expression, typed_hir, semantics),
            })
            .collect();
        let expressions = typed_hir
            .expressions()
            .map(lower_expression)
            .collect::<Vec<_>>();
        Self {
            profile,
            bodies,
            global_initializers,
            state_expressions,
            expressions,
        }
    }

    pub fn profile(&self) -> crate::BuildProfile {
        self.profile
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

fn lower_expression(expression: &TypedExpression) -> Expression {
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
                target: target.clone(),
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
                        MatchPattern::Enum {
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
                        MatchPattern::Bool(value) => LoweredPattern::Bool(*value),
                        MatchPattern::Int { value, .. } => LoweredPattern::Int(*value),
                        MatchPattern::None => {
                            let Some(ResolvedWrapperPattern::OptionNone(option)) =
                                arm.resolution.wrapper
                            else {
                                unreachable!("checked None patterns resolve to Options")
                            };
                            LoweredPattern::OptionNone(option)
                        }
                        MatchPattern::OptionSome(binding) => {
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
                        MatchPattern::ResultSuccess(binding) => {
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
                        MatchPattern::ResultError(binding) => {
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
                        MatchPattern::Wildcard => LoweredPattern::Wildcard,
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

fn lower_body(
    owner: BodyOwner,
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Body {
    let mut entry = if owner == BodyOwner::Action(ActionKind::OnAttach) {
        lower_async_block(block, typed_hir, semantics, profile)
    } else {
        lower_block(block, typed_hir, semantics, profile)
    };
    let mut next_async_state = 1;
    assign_async_states(&mut entry, &mut next_async_state);
    let locals = plan_block(block, typed_hir, semantics, profile);
    let frame_values = plan_frame_values(&mut entry, &locals, typed_hir);
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

fn plan_frame_values(
    entry: &mut Block,
    locals: &[Local],
    typed_hir: &TypedProgram,
) -> Vec<ValueId> {
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
        typed_hir,
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
    typed_hir: &TypedProgram,
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
                typed_hir,
                frame_values,
            );
            let mut suspension_live = continuation_live.clone();
            collect_expression_values(*value, &mut suspension_live, local_values, typed_hir);
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
            collect_expression_values(*value, &mut before_suspend, local_values, typed_hir);
            before_suspend
        }
        Terminator::Return(value) => {
            let mut live = HashSet::new();
            if let Some(value) = value {
                collect_expression_values(*value, &mut live, local_values, typed_hir);
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
                typed_hir,
                frame_values,
            );
            let mut loop_live = continuation_live;
            collect_expression_values(*condition, &mut loop_live, local_values, typed_hir);
            loop {
                let body_live = analyze_suspension_liveness(
                    body,
                    loop_live.clone(),
                    local_values,
                    ordered_locals,
                    typed_hir,
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
            collect_expression_values(*error, &mut live, local_values, typed_hir);
            live
        }
        Terminator::Fallthrough => live_after,
    };
    analyze_statements_liveness(
        &mut block.statements,
        &mut live,
        local_values,
        ordered_locals,
        typed_hir,
        frame_values,
    );
    live
}

fn analyze_statements_liveness(
    statements: &mut [Statement],
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    ordered_locals: &[Local],
    typed_hir: &TypedProgram,
    frame_values: &mut HashSet<ValueId>,
) {
    for statement in statements.iter_mut().rev() {
        match statement {
            Statement::Store { target, op, value } => {
                live.remove(target);
                if op.is_some() && local_values.contains(target) {
                    live.insert(*target);
                }
                collect_expression_values(*value, live, local_values, typed_hir);
            }
            Statement::Evaluate { expression, .. } => {
                collect_expression_values(*expression, live, local_values, typed_hir);
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
                    typed_hir,
                    frame_values,
                );
                let else_live = analyze_suspension_liveness(
                    else_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    typed_hir,
                    frame_values,
                );
                then_live.extend(else_live);
                collect_expression_values(*condition, &mut then_live, local_values, typed_hir);
                *live = then_live;
            }
            Statement::While { condition, body } => {
                let mut body_live = analyze_suspension_liveness(
                    body,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    typed_hir,
                    frame_values,
                );
                body_live.extend(live.iter().copied());
                collect_expression_values(*condition, &mut body_live, local_values, typed_hir);
                *live = body_live;
            }
        }
    }
}

fn collect_expression_values(
    expression: ExprId,
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    typed_hir: &TypedProgram,
) {
    struct Collector<'a> {
        live: &'a mut HashSet<ValueId>,
        local_values: &'a HashSet<ValueId>,
    }

    impl TypedVisitor for Collector<'_> {
        fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
            if let Some((Some(ResolvedValue::Variable(value)), _)) =
                program.value_path(expression.id)
                && self.local_values.contains(&value)
            {
                self.live.insert(value);
            }
            if let Some(call) = program.call(expression.id) {
                let receiver = match call {
                    ResolvedCall::UserMethod { receiver, .. } => Some(*receiver),
                    ResolvedCall::StandardLibrary { receiver, .. } => *receiver,
                    ResolvedCall::UserFunction { .. }
                    | ResolvedCall::ResultError { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::ResultSuccess { .. } => None,
                };
                if let Some(ResolvedValue::Variable(value)) = receiver
                    && self.local_values.contains(&value)
                {
                    self.live.insert(value);
                }
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    Collector { live, local_values }.visit_expression(
        typed_hir
            .expression(expression)
            .expect("lowered expressions belong to typed HIR"),
        typed_hir,
    );
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
    (StandardLibrary::new()
        .item(*item)
        .operation_semantics()
        .cancellation
        == CancellationKind::ProcessClose)
        .then_some(CancellationRegion::ProcessLifetime)
}

fn plan_block(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Vec<Local> {
    let mut planner = LocalPlanner::new(semantics, profile);
    planner.visit_block(block, typed_hir);
    planner.locals
}

fn plan_expression(
    expression: ExprId,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<Local> {
    let mut planner = LocalPlanner::new(semantics, crate::BuildProfile::Debug);
    planner.visit_expression(
        typed_hir
            .expression(expression)
            .expect("state source belongs to typed HIR"),
        typed_hir,
    );
    planner.locals
}

struct LocalPlanner<'a> {
    semantics: &'a SemanticModel,
    profile: crate::BuildProfile,
    locals: Vec<Local>,
}

impl<'a> LocalPlanner<'a> {
    fn new(semantics: &'a SemanticModel, profile: crate::BuildProfile) -> Self {
        Self {
            semantics,
            profile,
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
}

impl TypedVisitor for LocalPlanner<'_> {
    fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
        if statement.debug_only && self.profile == crate::BuildProfile::Release {
            return;
        }
        match statement.kind {
            TypedStatementKind::Variable { value, .. } => self.value(value),
            TypedStatementKind::Suspend {
                mode,
                binding,
                value,
            } => {
                if let Some(binding) = binding {
                    self.value(binding);
                }
                if mode == SuspensionMode::Retry {
                    let ty = program
                        .expression(value)
                        .expect("retried expression belongs to typed HIR")
                        .ty;
                    self.push(ty, LocalPurpose::SuspensionScratch(value));
                }
            }
            _ => {}
        }
        hir::walk_typed_statement(self, statement, program);
    }

    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        if let TypedExpressionKind::Match { value, arms } = &expression.kind {
            self.visit_expression(
                program
                    .expression(*value)
                    .expect("match input belongs to typed HIR"),
                program,
            );
            let value_type = program
                .expression(*value)
                .expect("match input belongs to typed HIR")
                .ty;
            self.push(value_type, LocalPurpose::MatchValue(expression.id));
            for arm in arms {
                let binding = match &arm.pattern {
                    MatchPattern::Enum {
                        binding: Some(binding),
                        ..
                    }
                    | MatchPattern::OptionSome(Some(binding))
                    | MatchPattern::ResultSuccess(Some(binding))
                    | MatchPattern::ResultError(Some(binding)) => Some(binding.id),
                    _ => None,
                };
                if let Some(binding) = binding {
                    let binding_type = self
                        .semantics
                        .value_type(binding)
                        .expect("checked pattern bindings have types");
                    self.push(binding_type, LocalPurpose::MatchBinding(arm.resolution.id));
                }
                if let Some(guard) = arm.guard {
                    self.visit_expression(
                        program
                            .expression(guard)
                            .expect("match guard belongs to typed HIR"),
                        program,
                    );
                }
                self.visit_expression(
                    program
                        .expression(arm.value)
                        .expect("match arm value belongs to typed HIR"),
                    program,
                );
            }
            return;
        }

        if let TypedExpressionKind::Fallback { value, fallback } = &expression.kind {
            self.visit_expression(
                program
                    .expression(*value)
                    .expect("fallback input belongs to typed HIR"),
                program,
            );
            let value_type = program
                .expression(*value)
                .expect("fallback input belongs to typed HIR")
                .ty;
            self.push(value_type, LocalPurpose::FallbackValue(expression.id));
            match fallback {
                TypedFallbackBranch::Value(fallback) => self.visit_expression(
                    program
                        .expression(*fallback)
                        .expect("fallback value belongs to typed HIR"),
                    program,
                ),
                TypedFallbackBranch::Return(Some(value)) => self.visit_expression(
                    program
                        .expression(*value)
                        .expect("fallback return value belongs to typed HIR"),
                    program,
                ),
                TypedFallbackBranch::Return(None)
                | TypedFallbackBranch::Break
                | TypedFallbackBranch::Continue => {}
            }
            return;
        }

        if let TypedExpressionKind::Propagate { value, .. } = expression.kind {
            self.visit_expression(
                program
                    .expression(value)
                    .expect("propagated input belongs to typed HIR"),
                program,
            );
            let input_type = program
                .expression(value)
                .expect("propagated input belongs to typed HIR")
                .ty;
            self.push(input_type, LocalPurpose::FallbackValue(expression.id));
            return;
        }

        hir::walk_typed_expression(self, expression, program);
        if let TypedExpressionKind::Call { .. } = expression.kind
            && let Some(intrinsic) = resolved_intrinsic(program, expression.id)
        {
            let scratch_types = match intrinsic {
                IntrinsicId::NumericClamp => vec![expression.ty; 4],
                IntrinsicId::NumericMin | IntrinsicId::NumericMax => vec![expression.ty; 2],
                IntrinsicId::TimerState => {
                    vec![self.semantics.types().id_for_builtin(BuiltinType::U32)]
                }
                IntrinsicId::ProcessFollow
                | IntrinsicId::ProcessReadRelative32
                | IntrinsicId::ProcessReadManagedString => {
                    let TypeKind::Result { value, .. } = self.semantics.types().kind(expression.ty)
                    else {
                        unreachable!("fallible process helpers return Result values")
                    };
                    vec![*value]
                }
                _ => Vec::new(),
            };
            for (slot, ty) in scratch_types.into_iter().enumerate() {
                self.push(
                    ty,
                    LocalPurpose::IntrinsicScratch {
                        expression: expression.id,
                        slot: slot as u8,
                    },
                );
            }
        }
    }
}

fn resolved_intrinsic(program: &TypedProgram, expression: ExprId) -> Option<IntrinsicId> {
    let ResolvedCall::StandardLibrary { item, .. } = program.call(expression)? else {
        return None;
    };
    match StandardLibrary::new().item(*item).implementation {
        Implementation::Intrinsic(intrinsic) => Some(intrinsic),
    }
}
