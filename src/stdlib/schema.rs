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
    /// An ordinary SplitScript function injected into the compilation unit.
    /// The generated name is reserved and cannot be authored by user code.
    LibraryBody {
        function_name: &'static str,
        /// The authored block, including its braces. The compiler injects one
        /// hidden function template and infers any catalog-generic portions of
        /// its signature through the ordinary function pipeline. A resolved
        /// catalog call later supplies the exact concrete signature used for
        /// demand-driven specialization.
        body: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Pure,
    Allocates,
    MutatesValue,
    ReadsTimer,
    ReadsRuntime,
    ReadsProcess,
    RequiresAttachedProcess,
    Retryable,
    Suspends,
    CancelsOnProcessClose,
    WritesTimer,
    WritesRuntime,
}

impl Effect {
    const ALL: [Self; 12] = [
        Self::Pure,
        Self::Allocates,
        Self::MutatesValue,
        Self::ReadsTimer,
        Self::ReadsRuntime,
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
            Self::ReadsRuntime => "reads runtime state",
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

    pub const fn without(self, effect: Effect) -> Self {
        Self(self.0 & !effect.bit())
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
    FixedArray {
        element: &'static TypeRef,
        length: u32,
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
    Method { receiver: TypeRef },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature {
    pub type_parameters: &'static [TypeParameter],
    /// Number of type parameters written on the callable itself. Remaining
    /// parameters are inherited from its receiver and are inferred from it.
    pub explicit_type_parameters: usize,
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

/// Complete catalog metadata from which normalized operation semantics are
/// derived. Intrinsics declare this metadata at the trusted boundary, while
/// source-defined functions receive it from compiler analysis of their bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationMetadata {
    pub effects: EffectSet,
    pub availability: Availability,
}

impl OperationMetadata {
    /// Conservatively combines alternative implementations of one public
    /// operation. `Pure` represents the absence of observable effects, so it
    /// is retained only when every alternative is pure.
    pub fn conservative_union(self, other: Self) -> Self {
        let effects = EffectSet(self.effects.0 | other.effects.0).without(Effect::Pure);
        let effects = if effects.is_empty() {
            EffectSet::one(Effect::Pure)
        } else {
            effects
        };
        Self {
            effects,
            availability: if self.availability == Availability::OnAttach
                || other.availability == Availability::OnAttach
            {
                Availability::OnAttach
            } else {
                Availability::Everywhere
            },
        }
    }

    pub fn semantics(self) -> OperationSemantics {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deprecation {
    pub message: &'static str,
    pub replacement: Option<StdlibItemId>,
}

/// Source-language binary syntax implemented by an ordinary catalog method.
///
/// Keeping this identity in the backend-neutral schema lets parsing, type
/// checking, documentation, and lowering agree on one declaration without
/// teaching those stages about particular standard-library types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardBinaryOperator {
    Add,
    Subtract,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibItem {
    pub id: StdlibItemId,
    pub owner: StdlibOwner,
    pub name: &'static str,
    pub qualified_name: &'static str,
    pub kind: ItemKind,
    pub binary_operator: Option<StandardBinaryOperator>,
    pub signature: Signature,
    pub must_use: Option<&'static str>,
    pub deprecation: Option<Deprecation>,
    pub documentation: Documentation<StdlibSymbolId>,
    pub implementation: Implementation,
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

    #[test]
    fn operation_unions_treat_pure_as_the_empty_effect_alternative() {
        let pure = OperationMetadata {
            effects: EffectSet::one(Effect::Pure),
            availability: Availability::Everywhere,
        };
        let process = OperationMetadata {
            effects: EffectSet::one(Effect::ReadsProcess),
            availability: Availability::OnAttach,
        };
        let union = pure.conservative_union(process);
        assert_eq!(
            union.effects.iter().copied().collect::<Vec<_>>(),
            [Effect::ReadsProcess]
        );
        assert_eq!(union.availability, Availability::OnAttach);
    }
}
