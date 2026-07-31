//! Dependency-light callable schema for standard-library declarations.
//!
//! These types describe signatures, effects, availability, and trusted
//! intrinsic bindings without depending on authored catalog data, the graph,
//! inference, tooling, or WebAssembly lowering.

use crate::catalog::Documentation;

use super::{
    declarations::{CoreTypeId, StdlibOwner, StdlibSymbolId},
    ids::{IntrinsicId, StdlibCapabilityId, StdlibItemId, StdlibTypeId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Implementation {
    Intrinsic(IntrinsicId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Pure,
    Allocates,
    MutatesValue,
    ReadsTimer,
    ReadsProcess,
    RequiresAttachedProcess,
    Retryable,
    Suspends,
    CancelsOnProcessClose,
    WritesTimer,
    WritesRuntime,
}

impl Effect {
    const ALL: [Self; 11] = [
        Self::Pure,
        Self::Allocates,
        Self::MutatesValue,
        Self::ReadsTimer,
        Self::ReadsProcess,
        Self::RequiresAttachedProcess,
        Self::Retryable,
        Self::Suspends,
        Self::CancelsOnProcessClose,
        Self::WritesTimer,
        Self::WritesRuntime,
    ];

    const fn bit(self) -> u16 {
        1 << self as u16
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Allocates => "allocates",
            Self::MutatesValue => "mutates the receiver",
            Self::ReadsTimer => "reads timer state",
            Self::ReadsProcess => "reads process memory",
            Self::RequiresAttachedProcess => "requires an attached process",
            Self::Retryable => "retryable",
            Self::Suspends => "suspends",
            Self::CancelsOnProcessClose => "cancels when the process closes",
            Self::WritesTimer => "writes timer state",
            Self::WritesRuntime => "writes runtime state",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EffectSet(u16);

impl EffectSet {
    pub const fn none() -> Self {
        Self(0)
    }

    pub const fn one(effect: Effect) -> Self {
        Self(effect.bit())
    }

    pub const fn with(self, effect: Effect) -> Self {
        Self(self.0 | effect.bit())
    }

    pub const fn contains(self, effect: &Effect) -> bool {
        self.0 & effect.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = &'static Effect> {
        Effect::ALL
            .iter()
            .filter(move |effect| self.contains(effect))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
    Parameter(&'static str),
    Application {
        constructor: super::ids::StdlibTypeConstructorId,
        arguments: &'static [TypeRef],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeParameter {
    pub name: &'static str,
    pub constraints: &'static [StdlibCapabilityId],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterRule {
    Value,
    StringLiteral,
    SignatureLiteral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    pub name: &'static str,
    pub ty: TypeRef,
    pub rule: ParameterRule,
    pub documentation: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    TypedFunction { type_parameter: &'static str },
    Method { receiver: TypeRef },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature {
    pub type_parameters: &'static [TypeParameter],
    pub parameters: &'static [Parameter],
    pub result: TypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Everywhere,
    OnAttach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionKind {
    None,
    Retryable,
    Suspends,
}

impl SuspensionKind {
    pub const fn is_awaitable(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationKind {
    None,
    ProcessClose,
}

/// Normalized operational facts consumed by type checking, lowering,
/// documentation, and editor tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationSemantics {
    pub availability: Availability,
    pub suspension: SuspensionKind,
    pub requires_attached_process: bool,
    pub cancellation: CancellationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deprecation {
    pub message: &'static str,
    pub replacement: Option<StdlibItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibItem {
    pub id: StdlibItemId,
    pub owner: StdlibOwner,
    pub name: &'static str,
    pub qualified_name: &'static str,
    pub kind: ItemKind,
    pub signature: Signature,
    pub effects: EffectSet,
    pub availability: Availability,
    pub deprecation: Option<Deprecation>,
    pub documentation: Documentation<StdlibSymbolId>,
    pub implementation: Implementation,
}

impl StdlibItem {
    pub fn operation_semantics(self) -> OperationSemantics {
        let suspension = if self.effects.contains(&Effect::Suspends) {
            SuspensionKind::Suspends
        } else if self.effects.contains(&Effect::Retryable) {
            SuspensionKind::Retryable
        } else {
            SuspensionKind::None
        };
        OperationSemantics {
            availability: self.availability,
            suspension,
            requires_attached_process: self.effects.contains(&Effect::RequiresAttachedProcess),
            cancellation: if self.effects.contains(&Effect::CancelsOnProcessClose) {
                CancellationKind::ProcessClose
            } else {
                CancellationKind::None
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_sets_are_unique_and_iterate_in_canonical_order() {
        let effects = EffectSet::one(Effect::WritesRuntime)
            .with(Effect::ReadsProcess)
            .with(Effect::WritesRuntime)
            .with(Effect::RequiresAttachedProcess);

        assert_eq!(
            effects.iter().copied().collect::<Vec<_>>(),
            vec![
                Effect::ReadsProcess,
                Effect::RequiresAttachedProcess,
                Effect::WritesRuntime,
            ]
        );
    }
}
