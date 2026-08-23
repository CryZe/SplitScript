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
    /// A signature contract owned by a structurally implemented capability.
    /// It is never a callable catalog implementation; user-defined methods
    /// satisfying the contract are resolved and lowered directly.
    CapabilityRequirement,
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
    /// Alternative source bodies selected from the concrete capabilities of
    /// one type argument during backend specialization. The callable remains
    /// one public operation with one signature; these are implementation
    /// cases, not general-purpose user-visible overloads.
    LibraryOverloads {
        dispatch_parameter: usize,
        cases: &'static [LibraryOverloadCase],
    },
}

/// Whether a callable belongs to the authored language surface or is an
/// implementation detail available only while checking trusted standard-
/// library source bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemVisibility {
    Public,
    LibraryPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryOverloadCase {
    pub capability: StdlibCapabilityId,
    pub signature: Signature,
    pub function_name: &'static str,
    pub body: &'static str,
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
    RequiresStateSnapshots,
    WritesCurrentState,
    Suspends,
    CancelsOnProcessClose,
    WritesTimer,
    WritesRuntime,
}

impl Effect {
    const ALL: [Self; 13] = [
        Self::Pure,
        Self::Allocates,
        Self::MutatesValue,
        Self::ReadsTimer,
        Self::ReadsRuntime,
        Self::ReadsProcess,
        Self::RequiresAttachedProcess,
        Self::RequiresStateSnapshots,
        Self::WritesCurrentState,
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
            Self::RequiresStateSnapshots => "requires state snapshots",
            Self::WritesCurrentState => "writes current state",
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
    /// Whether calling the operation constructs an `async T` value. `result`
    /// stores the completed `T` so generic substitution remains uniform.
    pub result_is_async: bool,
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

/// Contextual requirements authored by a privileged intrinsic declaration.
///
/// Rust's closed intrinsic registry independently declares the same contract
/// and validates it. Frontend consumers use this source-owned value, while the
/// registry remains the trust boundary for backend lowering and host imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntrinsicContext {
    pub availability: Availability,
    pub requires_attached_process: bool,
    pub requires_state_snapshots: bool,
    pub cancellation: CancellationKind,
}

impl IntrinsicContext {
    pub const fn effects(self) -> EffectSet {
        let mut effects = EffectSet::none();
        if self.requires_attached_process {
            effects = effects.with(Effect::RequiresAttachedProcess);
        }
        if self.requires_state_snapshots {
            effects = effects.with(Effect::RequiresStateSnapshots);
        }
        if matches!(self.cancellation, CancellationKind::ProcessClose) {
            effects = effects.with(Effect::CancelsOnProcessClose);
        }
        effects
    }
}

/// Normalized operational facts consumed by type checking, lowering,
/// documentation, and editor tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationSemantics {
    pub availability: Availability,
    pub suspension: SuspensionKind,
    pub requires_attached_process: bool,
    pub requires_state_snapshots: bool,
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
        } else {
            SuspensionKind::None
        };
        OperationSemantics {
            availability: self.availability,
            suspension,
            requires_attached_process: self.effects.contains(&Effect::RequiresAttachedProcess),
            requires_state_snapshots: self.effects.contains(&Effect::RequiresStateSnapshots),
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
    Multiply,
    Divide,
    Remainder,
    BitOr,
    BitXor,
    BitAnd,
    ShiftLeft,
    ShiftRight,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

impl StandardBinaryOperator {
    /// Canonical source spelling used by documentation and editor tooling.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Remainder => "%",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::BitAnd => "&",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
        }
    }
}

/// Source-language unary syntax implemented by an ordinary catalog method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardUnaryOperator {
    Not,
    Negate,
}

impl StandardUnaryOperator {
    /// Canonical source spelling used by documentation and editor tooling.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Not => "!",
            Self::Negate => "-",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibItem {
    pub id: StdlibItemId,
    pub owner: StdlibOwner,
    pub visibility: ItemVisibility,
    pub name: &'static str,
    pub qualified_name: &'static str,
    pub kind: ItemKind,
    pub binary_operator: Option<StandardBinaryOperator>,
    pub unary_operator: Option<StandardUnaryOperator>,
    pub signature: Signature,
    pub must_use: Option<&'static str>,
    pub deprecation: Option<Deprecation>,
    pub documentation: Documentation<StdlibSymbolId>,
    /// Present only for compiler-implemented intrinsic declarations. Ordinary
    /// source bodies receive their operation metadata from semantic analysis.
    pub intrinsic_context: Option<IntrinsicContext>,
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

    #[test]
    fn intrinsic_context_contains_only_contextual_requirements() {
        let context = IntrinsicContext {
            availability: Availability::Everywhere,
            requires_attached_process: true,
            requires_state_snapshots: false,
            cancellation: CancellationKind::None,
        };

        assert!(!context.effects().contains(&Effect::Suspends));
        assert!(context.effects().contains(&Effect::RequiresAttachedProcess));
    }
}
