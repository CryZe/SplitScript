//! Hidden effect polymorphism for callable and iterable values.
//!
//! Source types deliberately stay compact: `(T) -> U` does not expose an
//! effect parameter. This analysis nevertheless keeps effects latent while a
//! callable is merely stored, returned, or embedded in an iterator adapter.
//! The effects are substituted only where that value is invoked or iterated.

use super::{
    FunctionOperationSemantics, LatentOperationKind, LatentParameterOperation, OperationAnalysis,
    function_semantics, implicit_display_callees, merge_availability,
};
use crate::{
    ast::{ExprId, FunctionId, Program, ValueId},
    hir::{
        ExpressionResolution, TypedBlock, TypedExpression, TypedExpressionKind, TypedProgram,
        TypedStatement, TypedStatementKind,
    },
    semantic::{
        DynamicCallCallee, ResolvedCall, ResolvedMember, ResolvedRecordFieldId, ResolvedValue,
        SemanticModel,
    },
    stdlib::{Availability, Effect, StdlibFieldId, StdlibItemId, StdlibTypeConstructorId},
    types::TypeKind,
};
use std::collections::HashMap;

const MAX_ABSTRACT_VALUES: usize = 32;
const MAX_FIXPOINT_ROUNDS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SymbolicValue {
    Unknown,
    Parameter {
        index: usize,
        fields: Vec<ResolvedMember>,
    },
    Closure {
        parameters: Vec<ValueId>,
        body: ExprId,
        captures: Vec<(ValueId, SymbolicValue)>,
    },
    NamedFunction(FunctionId),
    Record {
        constructor: Option<StdlibTypeConstructorId>,
        fields: Vec<(ResolvedRecordFieldId, SymbolicValue)>,
    },
    Union(Vec<SymbolicValue>),
}

impl SymbolicValue {
    fn union(values: impl IntoIterator<Item = Self>) -> Self {
        let mut flattened = Vec::new();
        for value in values {
            match value {
                Self::Unknown => {}
                Self::Union(values) => {
                    for value in values {
                        if value != Self::Unknown && !flattened.contains(&value) {
                            flattened.push(value);
                        }
                    }
                }
                value if !flattened.contains(&value) => flattened.push(value),
                _ => {}
            }
            if flattened.len() >= MAX_ABSTRACT_VALUES {
                return Self::Unknown;
            }
        }
        match flattened.len() {
            0 => Self::Unknown,
            1 => flattened.pop().unwrap(),
            _ => Self::Union(flattened),
        }
    }

    fn project(&self, fields: &[ResolvedMember]) -> Self {
        let mut value = self.clone();
        for field in fields {
            value = match value {
                Self::Parameter {
                    index,
                    fields: mut existing,
                } => {
                    existing.push(*field);
                    Self::Parameter {
                        index,
                        fields: existing,
                    }
                }
                Self::Record { fields, .. } => fields
                    .into_iter()
                    .find_map(|(candidate, value)| {
                        member_matches_field(*field, candidate).then_some(value)
                    })
                    .unwrap_or(Self::Unknown),
                Self::Union(values) => {
                    Self::union(values.into_iter().map(|value| value.project(&[*field])))
                }
                Self::Closure { .. } | Self::NamedFunction(_) | Self::Unknown => Self::Unknown,
            };
        }
        value
    }

    fn substitute(&self, arguments: &[SymbolicValue], depth: usize) -> Self {
        if depth > MAX_ABSTRACT_VALUES {
            return Self::Unknown;
        }
        match self {
            Self::Unknown => Self::Unknown,
            Self::Parameter { index, fields } => arguments
                .get(*index)
                .map_or(Self::Unknown, |value| value.project(fields)),
            Self::Closure {
                parameters,
                body,
                captures,
            } => Self::Closure {
                parameters: parameters.clone(),
                body: *body,
                captures: captures
                    .iter()
                    .map(|(id, value)| (*id, value.substitute(arguments, depth + 1)))
                    .collect(),
            },
            Self::NamedFunction(function) => Self::NamedFunction(*function),
            Self::Record {
                constructor,
                fields,
            } => Self::Record {
                constructor: *constructor,
                fields: fields
                    .iter()
                    .map(|(field, value)| (*field, value.substitute(arguments, depth + 1)))
                    .collect(),
            },
            Self::Union(values) => Self::union(
                values
                    .iter()
                    .map(|value| value.substitute(arguments, depth + 1)),
            ),
        }
    }
}

fn member_matches_field(member: ResolvedMember, field: ResolvedRecordFieldId) -> bool {
    matches!(
        (member, field),
        (ResolvedMember::RecordField(left), ResolvedRecordFieldId::Source(right)) if left == right
    ) || matches!(
        (member, field),
        (ResolvedMember::StandardField(left), ResolvedRecordFieldId::Standard(right)) if left == right
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionSummary {
    operation: FunctionOperationSemantics,
    returned: SymbolicValue,
}

impl Default for FunctionSummary {
    fn default() -> Self {
        Self {
            operation: FunctionOperationSemantics::default(),
            returned: SymbolicValue::Unknown,
        }
    }
}

#[derive(Default)]
struct Accumulator {
    effects: Vec<Effect>,
    availability: Option<Availability>,
    latent: Vec<LatentParameterOperation>,
}

impl Accumulator {
    fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    fn availability(&mut self, availability: Availability) {
        self.availability = Some(self.availability.map_or(availability, |current| {
            merge_availability(current, availability)
        }));
    }

    fn operation(&mut self, operation: &FunctionOperationSemantics) {
        self.effects.extend_from_slice(&operation.effects);
        self.availability(operation.availability);
        for latent in &operation.latent_parameter_operations {
            if !self.latent.contains(latent) {
                self.latent.push(latent.clone());
            }
        }
    }

    fn finish(mut self) -> FunctionOperationSemantics {
        let mut operation = function_semantics(
            self.effects,
            self.availability.unwrap_or(Availability::Everywhere),
        );
        self.latent
            .sort_by_key(|latent| (latent.parameter, latent.fields.len()));
        self.latent.dedup();
        operation.latent_parameter_operations = self.latent;
        operation
    }
}

struct Evaluator<'a> {
    syntax: &'a Program,
    program: &'a TypedProgram,
    semantics: &'a SemanticModel,
    capabilities: &'a crate::capabilities::CapabilityAnalysis,
    scoped_globals: &'a crate::scoped_globals::ScopedGlobalAnalysis,
    summaries: &'a [FunctionSummary],
    env: HashMap<ValueId, SymbolicValue>,
    accumulator: Accumulator,
    returns: Vec<SymbolicValue>,
    calls: Option<&'a mut HashMap<ExprId, FunctionOperationSemantics>>,
}

impl<'a> Evaluator<'a> {
    fn new(
        syntax: &'a Program,
        program: &'a TypedProgram,
        semantics: &'a SemanticModel,
        capabilities: &'a crate::capabilities::CapabilityAnalysis,
        scoped_globals: &'a crate::scoped_globals::ScopedGlobalAnalysis,
        summaries: &'a [FunctionSummary],
        env: HashMap<ValueId, SymbolicValue>,
    ) -> Self {
        Self {
            syntax,
            program,
            semantics,
            capabilities,
            scoped_globals,
            summaries,
            env,
            accumulator: Accumulator::default(),
            returns: Vec::new(),
            calls: None,
        }
    }

    fn with_calls(mut self, calls: &'a mut HashMap<ExprId, FunctionOperationSemantics>) -> Self {
        self.calls = Some(calls);
        self
    }

    fn block(&mut self, block: &TypedBlock) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &TypedStatement) {
        match &statement.kind {
            TypedStatementKind::Variable { value, initializer } => {
                let initialized = self.expression(*initializer);
                self.env.insert(*value, initialized);
            }
            TypedStatementKind::Assign {
                assignment, value, ..
            } => {
                let value = self.expression(*value);
                if let Some(call) = &assignment.operator {
                    self.apply_resolved_call(None, call, &[], false);
                }
                self.env.insert(assignment.target, value);
                if self.scoped_globals.is_attachment_global(assignment.target) {
                    self.accumulator.effect(Effect::RequiresAttachedProcess);
                }
            }
            TypedStatementKind::StateAssign { target, value, .. } => {
                self.expression(*target);
                self.expression(*value);
                self.accumulator.effect(Effect::WritesCurrentState);
            }
            TypedStatementKind::IndexAssign {
                assignment,
                target,
                value,
                ..
            } => {
                self.expression(*target);
                self.expression(*value);
                self.apply_resolved_call(None, &assignment.operator, &[], false);
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expression(*condition);
                self.branch_blocks(then_block, else_block.as_ref());
            }
            TypedStatementKind::While { condition, body } => {
                self.expression(*condition);
                self.block(body);
            }
            TypedStatementKind::For { iterable, body, .. } => {
                let iterable_expression = *iterable;
                let iterable = self.expression(*iterable);
                let operation = self.operation_for_iteration(&iterable);
                self.record_call(iterable_expression, &operation);
                self.accumulator.operation(&operation);
                self.block(body);
            }
            TypedStatementKind::Suspend { value, .. } => {
                self.accumulator
                    .effects
                    .extend([Effect::Suspends, Effect::CancelsOnProcessClose]);
                self.accumulator.availability(Availability::OnAttach);
                self.suspended_expression(*value);
            }
            TypedStatementKind::Expression(expression) => {
                self.expression(*expression);
            }
        }
    }

    fn branch_blocks(&mut self, then_block: &TypedBlock, else_block: Option<&TypedBlock>) {
        let original = self.env.clone();
        self.block(then_block);
        let then_env = self.env.clone();
        self.env = original.clone();
        if let Some(else_block) = else_block {
            self.block(else_block);
        }
        let else_env = self.env.clone();
        self.env = merge_environments(original, then_env, else_env);
    }

    fn suspended_expression(&mut self, expression: ExprId) -> SymbolicValue {
        let Some(typed) = self.program.expression(expression) else {
            return SymbolicValue::Unknown;
        };
        if let TypedExpressionKind::Call {
            receiver,
            arguments,
            ..
        } = &typed.kind
            && let Some(call) = self.program.call(expression)
        {
            let receiver = receiver.map(|receiver| self.expression(receiver));
            let arguments = arguments
                .iter()
                .map(|argument| self.expression(*argument))
                .collect::<Vec<_>>();
            self.accumulator.effect(Effect::Allocates);
            return self.apply_resolved_call(
                Some(expression),
                call,
                &receiver.into_iter().chain(arguments).collect::<Vec<_>>(),
                true,
            );
        }
        self.expression(expression)
    }

    fn expression(&mut self, id: ExprId) -> SymbolicValue {
        let Some(expression) = self.program.expression(id) else {
            return SymbolicValue::Unknown;
        };
        self.record_implicit_effects(expression);
        match &expression.kind {
            TypedExpressionKind::None
            | TypedExpressionKind::IteratorEnd
            | TypedExpressionKind::Bool(_)
            | TypedExpressionKind::Int { .. }
            | TypedExpressionKind::Float(_)
            | TypedExpressionKind::Char(_)
            | TypedExpressionKind::String(_)
            | TypedExpressionKind::Signature(_) => SymbolicValue::Unknown,
            TypedExpressionKind::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::hir::TypedInterpolatedPart::Expression { expression, .. } = part {
                        self.expression(*expression);
                    }
                }
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Array(values) => {
                for value in values {
                    self.expression(*value);
                }
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Range { start, end, .. } => {
                self.expression(*start);
                self.expression(*end);
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Block { statements, value } => {
                self.block(statements);
                value.map_or(SymbolicValue::Unknown, |value| self.expression(value))
            }
            TypedExpressionKind::Loop { body } => {
                self.block(body);
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Record { record, fields } => {
                let resolved = self.program.record_literal_fields(id).unwrap_or_default();
                let fields = fields
                    .iter()
                    .zip(resolved)
                    .map(|((_, expression), field)| (*field, self.expression(*expression)))
                    .collect();
                let constructor = match record {
                    crate::semantic::ResolvedRecordId::StandardConstructor(_) => {
                        match self.semantics.types().kind(expression.ty) {
                            TypeKind::Application { constructor, .. } => Some(*constructor),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                SymbolicValue::Record {
                    constructor,
                    fields,
                }
            }
            TypedExpressionKind::Enum { payload, .. } => payload
                .map(|payload| self.expression(payload))
                .unwrap_or(SymbolicValue::Unknown),
            TypedExpressionKind::Match { value, arms } => {
                self.expression(*value);
                SymbolicValue::union(arms.iter().map(|arm| {
                    if let Some(guard) = arm.guard {
                        self.expression(guard);
                    }
                    self.expression(arm.value)
                }))
            }
            TypedExpressionKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expression(*condition);
                SymbolicValue::union([self.expression(*then_expr), self.expression(*else_expr)])
            }
            TypedExpressionKind::Fallback { value, fallback } => {
                SymbolicValue::union([self.expression(*value), self.expression(*fallback)])
            }
            TypedExpressionKind::Break(value) | TypedExpressionKind::Return(value) => {
                let returned = value
                    .map(|value| self.expression(value))
                    .unwrap_or(SymbolicValue::Unknown);
                if matches!(expression.kind, TypedExpressionKind::Return(_)) {
                    self.returns.push(returned.clone());
                }
                returned
            }
            TypedExpressionKind::Continue => SymbolicValue::Unknown,
            TypedExpressionKind::Throw { error, .. } => {
                self.expression(*error);
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Suspend { value, .. } => {
                self.accumulator
                    .effects
                    .extend([Effect::Suspends, Effect::CancelsOnProcessClose]);
                self.accumulator.availability(Availability::OnAttach);
                self.suspended_expression(*value)
            }
            TypedExpressionKind::Propagate { value, .. } => self.expression(*value),
            TypedExpressionKind::Path(_) => match &expression.resolution {
                Some(ExpressionResolution::FunctionValue(function)) => {
                    SymbolicValue::NamedFunction(function.function)
                }
                _ => self.value_path(id),
            },
            TypedExpressionKind::Member { receiver, .. } => {
                let receiver = self.expression(*receiver);
                match &expression.resolution {
                    Some(ExpressionResolution::Member { members }) => receiver.project(members),
                    _ => self.value_path(id),
                }
            }
            TypedExpressionKind::Index { receiver, index } => {
                self.expression(*receiver);
                self.expression(*index);
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Unary { expression, .. }
            | TypedExpressionKind::Cast { expression, .. } => self.expression(*expression),
            TypedExpressionKind::Binary { left, right, .. } => {
                self.expression(*left);
                self.expression(*right);
                SymbolicValue::Unknown
            }
            TypedExpressionKind::Call {
                receiver,
                arguments,
                ..
            } => {
                let receiver = receiver.map(|receiver| self.expression(receiver));
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(*argument))
                    .collect::<Vec<_>>();
                let inputs = receiver.into_iter().chain(arguments).collect::<Vec<_>>();
                let creates_future = matches!(
                    self.semantics.types().kind(expression.ty),
                    TypeKind::Async { .. }
                );
                if creates_future {
                    self.accumulator.effect(Effect::Allocates);
                }
                if let Some(call) = self.program.call(id) {
                    self.apply_resolved_call(Some(id), call, &inputs, !creates_future)
                } else if let Some(ExpressionResolution::DynamicCall(callee)) =
                    expression.resolution
                {
                    let callee = match callee {
                        DynamicCallCallee::Value(value) => self
                            .env
                            .get(&value)
                            .cloned()
                            .unwrap_or(SymbolicValue::Unknown),
                        DynamicCallCallee::Expression(expression) => self.expression(expression),
                    };
                    let (operation, returned) = self.operation_for_invoke(&callee, &inputs);
                    self.record_call(id, &operation);
                    self.accumulator.operation(&operation);
                    returned
                } else {
                    SymbolicValue::Unknown
                }
            }
            TypedExpressionKind::Invoke { callee, arguments } => {
                let callee = self.expression(*callee);
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expression(*argument))
                    .collect::<Vec<_>>();
                let (operation, returned) = self.operation_for_invoke(&callee, &arguments);
                self.record_call(id, &operation);
                self.accumulator.operation(&operation);
                returned
            }
            TypedExpressionKind::Closure { parameters, body } => SymbolicValue::Closure {
                parameters: parameters.clone(),
                body: *body,
                captures: self
                    .env
                    .iter()
                    .map(|(id, value)| (*id, value.clone()))
                    .collect(),
            },
        }
    }

    fn record_implicit_effects(&mut self, expression: &TypedExpression) {
        for function in
            implicit_display_callees(expression, self.program, self.semantics, self.capabilities)
        {
            if let Some(summary) = self.summaries.get(function.index()) {
                self.accumulator.operation(&summary.operation);
            }
        }
        if let Some((root, _)) = self.program.value_path(expression.id) {
            if matches!(
                root,
                Some(
                    ResolvedValue::CurrentSnapshot
                        | ResolvedValue::OldSnapshot
                        | ResolvedValue::CurrentState(_)
                        | ResolvedValue::OldState(_)
                )
            ) {
                self.accumulator.effect(Effect::RequiresStateSnapshots);
            }
            if let Some(ResolvedValue::Variable(value)) = root
                && self.scoped_globals.is_attachment_global(value)
            {
                self.accumulator.effect(Effect::RequiresAttachedProcess);
            }
        }
    }

    fn value_path(&self, id: ExprId) -> SymbolicValue {
        let Some((root, members)) = self.program.value_path(id) else {
            return SymbolicValue::Unknown;
        };
        let Some(root) = root.and_then(ResolvedValue::source_value) else {
            return SymbolicValue::Unknown;
        };
        self.env
            .get(&root)
            .map_or(SymbolicValue::Unknown, |value| value.project(members))
    }

    fn apply_resolved_call(
        &mut self,
        expression: Option<ExprId>,
        call: &ResolvedCall,
        inputs: &[SymbolicValue],
        execute: bool,
    ) -> SymbolicValue {
        if call
            .receiver()
            .and_then(|receiver| receiver.path().map(|(root, _)| root))
            .and_then(ResolvedValue::source_value)
            .is_some_and(|value| self.scoped_globals.is_attachment_global(value))
        {
            self.accumulator.effect(Effect::RequiresAttachedProcess);
        }

        if let ResolvedCall::StandardLibrary { item, .. } = call {
            if *item == StdlibItemId::IterableIterator {
                return inputs.first().cloned().unwrap_or(SymbolicValue::Unknown);
            }
            if *item == StdlibItemId::IteratorNext && execute {
                let value = inputs.first().cloned().unwrap_or(SymbolicValue::Unknown);
                let operation = self.operation_for_iteration(&value);
                if let Some(expression) = expression {
                    self.record_call(expression, &operation);
                }
                self.accumulator.operation(&operation);
                return SymbolicValue::Unknown;
            }
        }

        let Some(summary) = self.call_summary(call).cloned() else {
            let operation = intrinsic_operation(call, self.program, execute);
            if let Some(expression) = expression {
                self.record_call(expression, &operation);
            }
            self.accumulator.operation(&operation);
            return SymbolicValue::Unknown;
        };

        if !execute {
            self.accumulator
                .availability(summary.operation.availability);
            return SymbolicValue::Unknown;
        }
        let operation = self.instantiate_operation(&summary.operation, inputs);
        if let Some(expression) = expression {
            self.record_call(expression, &operation);
        }
        self.accumulator.operation(&operation);
        summary.returned.substitute(inputs, 0)
    }

    fn call_summary(&self, call: &ResolvedCall) -> Option<&FunctionSummary> {
        let function = match call {
            ResolvedCall::UserFunction { function, .. }
            | ResolvedCall::UserMethod { function, .. } => Some(*function),
            ResolvedCall::StandardLibrary { item, .. } => self.program.library_function(*item),
            ResolvedCall::ManagedSnapshot { .. } => None,
            ResolvedCall::ResultError { .. }
            | ResolvedCall::OptionSome { .. }
            | ResolvedCall::IteratorItem { .. }
            | ResolvedCall::ResultSuccess { .. } => None,
        }?;
        self.summaries.get(function.index())
    }

    fn instantiate_operation(
        &mut self,
        operation: &FunctionOperationSemantics,
        inputs: &[SymbolicValue],
    ) -> FunctionOperationSemantics {
        let mut accumulator = Accumulator::default();
        accumulator.effects.extend_from_slice(&operation.effects);
        accumulator.availability(operation.availability);
        for latent in &operation.latent_parameter_operations {
            let value = inputs
                .get(latent.parameter)
                .map_or(SymbolicValue::Unknown, |value| {
                    value.project(&latent.fields)
                });
            let nested = match latent.kind {
                LatentOperationKind::Invoke => self.operation_for_invoke(&value, &[]).0,
                LatentOperationKind::Iterate => self.operation_for_iteration(&value),
            };
            accumulator.operation(&nested);
        }
        accumulator.finish()
    }

    fn operation_for_invoke(
        &mut self,
        callee: &SymbolicValue,
        arguments: &[SymbolicValue],
    ) -> (FunctionOperationSemantics, SymbolicValue) {
        match callee {
            SymbolicValue::Parameter { index, fields } => {
                let mut operation = FunctionOperationSemantics::default();
                operation.latent_parameter_operations = vec![LatentParameterOperation {
                    parameter: *index,
                    fields: fields.clone(),
                    kind: LatentOperationKind::Invoke,
                }];
                (operation, SymbolicValue::Unknown)
            }
            SymbolicValue::Closure {
                parameters,
                body,
                captures,
            } => {
                let mut env = captures.iter().cloned().collect::<HashMap<_, _>>();
                for (parameter, argument) in parameters.iter().zip(arguments) {
                    env.insert(*parameter, argument.clone());
                }
                let mut child = Evaluator::new(
                    self.syntax,
                    self.program,
                    self.semantics,
                    self.capabilities,
                    self.scoped_globals,
                    self.summaries,
                    env,
                );
                let returned = child.expression(*body);
                let returned = SymbolicValue::union(child.returns.into_iter().chain([returned]));
                (child.accumulator.finish(), returned)
            }
            SymbolicValue::NamedFunction(function) => {
                let Some(summary) = self.summaries.get(function.index()) else {
                    return (
                        FunctionOperationSemantics::default(),
                        SymbolicValue::Unknown,
                    );
                };
                let operation = self.instantiate_operation(&summary.operation, arguments);
                let returned = summary.returned.substitute(arguments, 0);
                (operation, returned)
            }
            SymbolicValue::Union(values) => {
                let mut accumulator = Accumulator::default();
                let mut returned = Vec::new();
                for value in values {
                    let (operation, value) = self.operation_for_invoke(value, arguments);
                    accumulator.operation(&operation);
                    returned.push(value);
                }
                (accumulator.finish(), SymbolicValue::union(returned))
            }
            SymbolicValue::Record { .. } | SymbolicValue::Unknown => (
                FunctionOperationSemantics::default(),
                SymbolicValue::Unknown,
            ),
        }
    }

    fn operation_for_iteration(&mut self, value: &SymbolicValue) -> FunctionOperationSemantics {
        match value {
            SymbolicValue::Parameter { index, fields } => {
                let mut operation = FunctionOperationSemantics::default();
                operation.latent_parameter_operations = vec![LatentParameterOperation {
                    parameter: *index,
                    fields: fields.clone(),
                    kind: LatentOperationKind::Iterate,
                }];
                operation
            }
            SymbolicValue::Record {
                constructor: Some(StdlibTypeConstructorId::MapIterator),
                fields,
            } => {
                let source = record_field(fields, StdlibFieldId::MapIteratorSource);
                let transform = record_field(fields, StdlibFieldId::MapIteratorTransform);
                let mut accumulator = Accumulator::default();
                accumulator.operation(&self.operation_for_iteration(&source));
                accumulator.operation(&self.operation_for_invoke(&transform, &[]).0);
                accumulator.finish()
            }
            SymbolicValue::Record {
                constructor: Some(StdlibTypeConstructorId::FilterIterator),
                fields,
            } => {
                let source = record_field(fields, StdlibFieldId::FilterIteratorSource);
                let predicate = record_field(fields, StdlibFieldId::FilterIteratorPredicate);
                let mut accumulator = Accumulator::default();
                accumulator.operation(&self.operation_for_iteration(&source));
                accumulator.operation(&self.operation_for_invoke(&predicate, &[]).0);
                accumulator.finish()
            }
            SymbolicValue::Union(values) => {
                let mut accumulator = Accumulator::default();
                for value in values {
                    accumulator.operation(&self.operation_for_iteration(value));
                }
                accumulator.finish()
            }
            SymbolicValue::Closure { .. }
            | SymbolicValue::NamedFunction(_)
            | SymbolicValue::Record { .. }
            | SymbolicValue::Unknown => FunctionOperationSemantics::default(),
        }
    }

    fn record_call(&mut self, expression: ExprId, operation: &FunctionOperationSemantics) {
        if let Some(calls) = self.calls.as_deref_mut() {
            calls.insert(expression, operation.clone());
        }
    }
}

fn record_field(
    fields: &[(ResolvedRecordFieldId, SymbolicValue)],
    expected: StdlibFieldId,
) -> SymbolicValue {
    fields
        .iter()
        .find_map(|(field, value)| {
            (*field == ResolvedRecordFieldId::Standard(expected)).then(|| value.clone())
        })
        .unwrap_or(SymbolicValue::Unknown)
}

fn merge_environments(
    original: HashMap<ValueId, SymbolicValue>,
    then_env: HashMap<ValueId, SymbolicValue>,
    else_env: HashMap<ValueId, SymbolicValue>,
) -> HashMap<ValueId, SymbolicValue> {
    let mut merged = original;
    for key in then_env.keys().chain(else_env.keys()) {
        let value = SymbolicValue::union([
            then_env.get(key).cloned().unwrap_or(SymbolicValue::Unknown),
            else_env.get(key).cloned().unwrap_or(SymbolicValue::Unknown),
        ]);
        merged.insert(*key, value);
    }
    merged
}

fn intrinsic_operation(
    call: &ResolvedCall,
    program: &TypedProgram,
    execute: bool,
) -> FunctionOperationSemantics {
    let mut accumulator = Accumulator::default();
    if let ResolvedCall::StandardLibrary { item, .. } = call {
        let metadata = program.standard_library().operation_metadata(*item);
        if execute {
            accumulator.effects.extend(metadata.effects.iter().copied());
        }
        accumulator.availability(metadata.availability);
    } else if matches!(call, ResolvedCall::ManagedSnapshot { .. }) {
        accumulator.effect(Effect::RequiresAttachedProcess);
    }
    accumulator.finish()
}

fn function_environment(function: FunctionId, syntax: &Program) -> HashMap<ValueId, SymbolicValue> {
    syntax
        .functions
        .iter()
        .find(|candidate| candidate.id == function)
        .map(|function| {
            function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| {
                    (
                        parameter.id,
                        SymbolicValue::Parameter {
                            index,
                            fields: Vec::new(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn evaluate_function(
    function: FunctionId,
    body: &TypedBlock,
    syntax: &Program,
    program: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
    scoped_globals: &crate::scoped_globals::ScopedGlobalAnalysis,
    summaries: &[FunctionSummary],
) -> FunctionSummary {
    let mut evaluator = Evaluator::new(
        syntax,
        program,
        semantics,
        capabilities,
        scoped_globals,
        summaries,
        function_environment(function, syntax),
    );
    evaluator.block(body);
    FunctionSummary {
        operation: evaluator.accumulator.finish(),
        returned: SymbolicValue::union(evaluator.returns),
    }
}

pub(super) fn infer(
    syntax: &Program,
    program: &TypedProgram,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
    scoped_globals: &crate::scoped_globals::ScopedGlobalAnalysis,
) -> OperationAnalysis {
    let function_count = program
        .all_function_bodies()
        .map(|body| body.function.function.index() + 1)
        .max()
        .unwrap_or(0);
    let mut summaries = vec![FunctionSummary::default(); function_count];
    for _ in 0..MAX_FIXPOINT_ROUNDS {
        let mut next = summaries.clone();
        for body in program.all_function_bodies() {
            next[body.function.function.index()] = evaluate_function(
                body.function.function,
                &body.body,
                syntax,
                program,
                semantics,
                capabilities,
                scoped_globals,
                &summaries,
            );
        }
        if next == summaries {
            summaries = next;
            break;
        }
        summaries = next;
    }

    let mut calls = HashMap::new();
    for action in program.action_bodies() {
        let mut evaluator = Evaluator::new(
            syntax,
            program,
            semantics,
            capabilities,
            scoped_globals,
            &summaries,
            HashMap::new(),
        )
        .with_calls(&mut calls);
        evaluator.block(&action.body);
    }
    for (_, expression) in program.state_sources() {
        let mut evaluator = Evaluator::new(
            syntax,
            program,
            semantics,
            capabilities,
            scoped_globals,
            &summaries,
            HashMap::new(),
        )
        .with_calls(&mut calls);
        evaluator.expression(expression);
    }
    for transform in program.state_transforms() {
        let mut evaluator = Evaluator::new(
            syntax,
            program,
            semantics,
            capabilities,
            scoped_globals,
            &summaries,
            HashMap::new(),
        )
        .with_calls(&mut calls);
        evaluator.expression(transform.expression);
    }

    OperationAnalysis {
        functions: summaries
            .into_iter()
            .map(|summary| summary.operation)
            .collect(),
        calls,
    }
}
