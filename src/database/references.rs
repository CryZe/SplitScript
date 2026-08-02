//! Identity-based value reference indexing for checked programs.

use std::{collections::HashMap, sync::Arc};

use crate::{
    CheckedProgram,
    ast::{AssignmentId, ExprId, Span, ValueId},
    hir::ResolvedAssignment,
    semantic::{ResolvedCall, ResolvedValue},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueReferenceKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueReference {
    pub target: ValueId,
    pub kind: ValueReferenceKind,
    pub span: Span,
    pub expression: Option<ExprId>,
    pub assignment: Option<AssignmentId>,
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceIndex {
    by_value: HashMap<ValueId, Arc<[ValueReference]>>,
}

impl ReferenceIndex {
    pub(super) fn build(checked: &CheckedProgram) -> Self {
        let mut references = HashMap::<ValueId, Vec<ValueReference>>::new();
        for expression in checked.typed_hir().expressions() {
            let read = checked
                .typed_hir()
                .value_path(expression.id)
                .and_then(|(root, _)| root)
                .or_else(|| {
                    checked
                        .typed_hir()
                        .call(expression.id)
                        .and_then(call_receiver)
                });
            if let Some(read) = read {
                let Some(target) = resolved_value_id(read) else {
                    continue;
                };
                references.entry(target).or_default().push(ValueReference {
                    target,
                    kind: ValueReferenceKind::Read,
                    span: expression.span,
                    expression: Some(expression.id),
                    assignment: None,
                });
            }
        }
        for (assignment, target) in checked.semantics().assignment_targets() {
            let Some(ResolvedAssignment { span, .. }) = checked.typed_hir().assignment(assignment)
            else {
                continue;
            };
            references.entry(target).or_default().push(ValueReference {
                target,
                kind: ValueReferenceKind::Write,
                span,
                expression: None,
                assignment: Some(assignment),
            });
        }
        let by_value = references
            .into_iter()
            .map(|(value, mut references)| {
                references.sort_by_key(|reference| {
                    (reference.span.start, reference.span.end, reference.kind)
                });
                (value, Arc::from(references))
            })
            .collect();
        Self { by_value }
    }

    pub fn references_to(&self, value: ValueId) -> &[ValueReference] {
        self.by_value.get(&value).map_or(&[], AsRef::as_ref)
    }

    pub fn referenced_values(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.by_value.keys().copied()
    }
}

pub(super) fn resolved_value_id(value: ResolvedValue) -> Option<ValueId> {
    match value {
        ResolvedValue::ProviderValue(_) => None,
        ResolvedValue::Variable(value)
        | ResolvedValue::CurrentState(value)
        | ResolvedValue::OldState(value)
        | ResolvedValue::Setting(value)
        | ResolvedValue::OldSetting(value) => Some(value),
    }
}

fn call_receiver(call: &ResolvedCall) -> Option<ResolvedValue> {
    match call {
        ResolvedCall::UserMethod { receiver, .. } => receiver.path().map(|(root, _)| root),
        ResolvedCall::StandardLibrary { receiver, .. } => receiver
            .as_ref()
            .and_then(|receiver| receiver.path().map(|(root, _)| root)),
        ResolvedCall::UserFunction { .. }
        | ResolvedCall::ResultError { .. }
        | ResolvedCall::OptionSome { .. }
        | ResolvedCall::ResultSuccess { .. } => None,
    }
}
