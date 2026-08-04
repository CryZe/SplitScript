//! Trusted contracts for compiler-implemented standard-library operations.
//!
//! Public declarations are authored separately. This registry is deliberately
//! small and closed: it describes what the compiler promises to implement and
//! lets catalog validation reject a privileged binding that understates or
//! reshapes that implementation.

use crate::{
    abi::AbiImportId,
    stdlib::{
        Availability, CoreTypeId, Effect, EffectSet, IntrinsicId, ItemKind, ParameterRule,
        Signature, StdlibCapabilityId, StdlibTypeConstructorId, StdlibTypeId, TypeRef,
    },
};

/// Hard upper bound for one native process-string read. Both direct calls and
/// state-field decoder sugar fail before asking the host to exceed this size.
pub(crate) const MAX_NATIVE_STRING_BYTES: u32 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallableShape {
    Function,
    Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoweringClass {
    HostBoundary,
    RepresentationPrimitive,
    Retryable,
    Suspension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum RuntimeHelperId {
    PrintString,
    TimerSetVariable,
    FormatI64,
    StringEquality,
    StringMatch,
    StringFind,
    StringReplaceAll,
    StringSlice,
    ScanProcessRange,
    ReadRelative32,
    StringFromMemory,
    ReadUtf8String,
    ReadManagedString,
    ModulePath,
    UnityAttach,
    CStringEquality,
    BackingFieldEquality,
    UnityGetImage,
    UnityGetClass,
    UnityGetFieldOffset,
    UnityGetFieldAny,
    UnityGetStaticInstance,
    ConcatStrings,
    FollowAddress,
    GbaAttach,
    GbaTranslateAddress,
    RefreshSettings,
    SettingsEnabled,
}

impl RuntimeHelperId {
    pub(crate) const COUNT: usize = Self::SettingsEnabled as usize + 1;

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DependencyRoot {
    Helper(RuntimeHelperId),
    HostImport(AbiImportId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScratchType {
    Core(CoreTypeId),
    Expression,
    ResultValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractTypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
    Parameter(u8),
    Application {
        constructor: StdlibTypeConstructorId,
        arguments: &'static [ContractTypeRef],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContractParameter {
    pub(crate) ty: ContractTypeRef,
    pub(crate) rule: ParameterRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IntrinsicSignature {
    pub(crate) type_parameter_constraints: Option<&'static [StdlibCapabilityId]>,
    pub(crate) receiver: Option<ContractTypeRef>,
    pub(crate) parameters: [Option<ContractParameter>; 3],
    pub(crate) result: ContractTypeRef,
}

impl IntrinsicSignature {
    pub(crate) fn matches(self, kind: ItemKind, signature: Signature) -> bool {
        if usize::from(self.type_parameter_constraints.is_some()) != signature.type_parameters.len()
            || self.parameters.iter().flatten().count() != signature.parameters.len()
        {
            return false;
        }
        if let Some(required) = self.type_parameter_constraints
            && signature.type_parameters[0].constraints != required
        {
            return false;
        }
        let names = signature
            .type_parameters
            .iter()
            .map(|parameter| parameter.name)
            .collect::<Vec<_>>();
        let declared_receiver = match kind {
            ItemKind::Method { receiver } => Some(receiver),
            ItemKind::Function => None,
        };
        if !matches_optional_type(self.receiver, declared_receiver, &names) {
            return false;
        }
        self.parameters
            .iter()
            .flatten()
            .zip(signature.parameters)
            .all(|(required, declared)| {
                required.rule == declared.rule && matches_type(required.ty, declared.ty, &names)
            })
            && matches_type(self.result, signature.result, &names)
    }
}

fn matches_optional_type(
    required: Option<ContractTypeRef>,
    declared: Option<TypeRef>,
    parameters: &[&str],
) -> bool {
    match (required, declared) {
        (Some(required), Some(declared)) => matches_type(required, declared, parameters),
        (None, None) => true,
        _ => false,
    }
}

fn matches_type(required: ContractTypeRef, declared: TypeRef, parameters: &[&str]) -> bool {
    match (required, declared) {
        (ContractTypeRef::Core(required), TypeRef::Core(declared)) => required == declared,
        (ContractTypeRef::Standard(required), TypeRef::Standard(declared)) => required == declared,
        (ContractTypeRef::Parameter(required), TypeRef::Parameter(declared)) => parameters
            .get(required as usize)
            .is_some_and(|name| *name == declared),
        (
            ContractTypeRef::Application {
                constructor: required_constructor,
                arguments: required_arguments,
            },
            TypeRef::Application {
                constructor: declared_constructor,
                arguments: declared_arguments,
            },
        ) => {
            required_constructor == declared_constructor
                && required_arguments.len() == declared_arguments.len()
                && required_arguments
                    .iter()
                    .zip(declared_arguments)
                    .all(|(required, declared)| matches_type(*required, *declared, parameters))
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScratchPolicy {
    pub(crate) ty: ScratchType,
    pub(crate) slots: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IntrinsicContract {
    pub(crate) id: IntrinsicId,
    pub(crate) shape: CallableShape,
    pub(crate) signature: IntrinsicSignature,
    pub(crate) effects: EffectSet,
    pub(crate) availability: Availability,
    pub(crate) lowering: LoweringClass,
    pub(crate) dependency_roots: &'static [DependencyRoot],
    pub(crate) async_scratch: Option<ScratchPolicy>,
    /// Values retained inside a compiler-generated future between polls.
    pub(crate) async_state: Option<ScratchPolicy>,
    pub(crate) synchronous_scratch: Option<ScratchPolicy>,
}

impl IntrinsicContract {
    const fn new(
        id: IntrinsicId,
        shape: CallableShape,
        signature: IntrinsicSignature,
        effects: EffectSet,
        availability: Availability,
        lowering: LoweringClass,
    ) -> Self {
        Self {
            id,
            shape,
            signature,
            effects,
            availability,
            lowering,
            dependency_roots: dependency_roots(id),
            async_scratch: async_scratch(id),
            async_state: async_state(id),
            synchronous_scratch: synchronous_scratch(id),
        }
    }

    pub(crate) const fn accepts(self, kind: ItemKind) -> bool {
        matches!(
            (self.shape, kind),
            (CallableShape::Function, ItemKind::Function)
                | (CallableShape::Method, ItemKind::Method { .. })
        )
    }
}

const fn scratch(ty: ScratchType, slots: u8) -> Option<ScratchPolicy> {
    Some(ScratchPolicy { ty, slots })
}

const fn async_scratch(id: IntrinsicId) -> Option<ScratchPolicy> {
    match id {
        IntrinsicId::ProcessMainModule | IntrinsicId::ProcessModule => {
            scratch(ScratchType::Core(CoreTypeId::U64), 2)
        }
        IntrinsicId::ProcessScan => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ProcessScanMemory => scratch(ScratchType::Core(CoreTypeId::U64), 7),
        IntrinsicId::ProcessScanMemoryAny => scratch(ScratchType::Core(CoreTypeId::U64), 8),
        IntrinsicId::ModuleScan => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ProcessFollow
        | IntrinsicId::ProcessReadRelative32
        | IntrinsicId::UnityClassField
        | IntrinsicId::UnityClassStaticInstance
        | IntrinsicId::UnityClassStaticTable => scratch(ScratchType::Core(CoreTypeId::U64), 1),
        IntrinsicId::UnityIl2Cpp
        | IntrinsicId::UnityModuleImage
        | IntrinsicId::UnityImageClass
        | IntrinsicId::UnityClassFieldAny => scratch(ScratchType::Expression, 1),
        _ => None,
    }
}

const fn async_state(id: IntrinsicId) -> Option<ScratchPolicy> {
    match id {
        // A completed scan is delivered on the poll after the bounded window
        // that found it. Besides making the operation observably async, the
        // extra result slots prevent chained scans from consuming several
        // windows during one host update.
        IntrinsicId::ProcessScan | IntrinsicId::ModuleScan => {
            scratch(ScratchType::Core(CoreTypeId::U64), 2)
        }
        IntrinsicId::ProcessScanMemory => scratch(ScratchType::Core(CoreTypeId::U64), 3),
        IntrinsicId::ProcessScanMemoryAny => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        _ => None,
    }
}

const fn synchronous_scratch(id: IntrinsicId) -> Option<ScratchPolicy> {
    match id {
        IntrinsicId::NumericMin | IntrinsicId::NumericMax => scratch(ScratchType::Expression, 2),
        IntrinsicId::TimerState => scratch(ScratchType::Core(CoreTypeId::U32), 1),
        IntrinsicId::ProcessFollow
        | IntrinsicId::ProcessReadRelative32
        | IntrinsicId::ProcessReadUtf8
        | IntrinsicId::ProcessReadManagedString
        | IntrinsicId::ModulePath
        | IntrinsicId::StringReplaceAll
        | IntrinsicId::StringSlice
        | IntrinsicId::GbaAttach => scratch(ScratchType::ResultValue, 1),
        IntrinsicId::GbaEmulatorRead => scratch(ScratchType::Core(CoreTypeId::Address), 1),
        _ => None,
    }
}

const fn dependency_roots(id: IntrinsicId) -> &'static [DependencyRoot] {
    use AbiImportId as Host;
    use DependencyRoot::{Helper, HostImport};
    use RuntimeHelperId as Runtime;

    match id {
        IntrinsicId::Print => &[Helper(Runtime::PrintString), Helper(Runtime::FormatI64)],
        IntrinsicId::TimerSetVariable => &[
            Helper(Runtime::TimerSetVariable),
            Helper(Runtime::FormatI64),
        ],
        IntrinsicId::RuntimeSetTickRate => &[HostImport(Host::RuntimeSetTickRate)],
        IntrinsicId::SettingsEnabled => &[Helper(Runtime::SettingsEnabled)],
        IntrinsicId::InstantNow => &[HostImport(Host::WasiClockTimeGet)],
        IntrinsicId::TimerState => &[HostImport(Host::TimerGetState)],
        IntrinsicId::TimerCurrentSplitIndex => &[HostImport(Host::TimerCurrentSplitIndex)],
        IntrinsicId::TimerPauseGameTime => &[HostImport(Host::TimerPauseGameTime)],
        IntrinsicId::TimerResumeGameTime => &[HostImport(Host::TimerResumeGameTime)],
        IntrinsicId::ProcessMainModule | IntrinsicId::ProcessModule => &[
            HostImport(Host::ProcessGetModuleAddress),
            HostImport(Host::ProcessGetModuleSize),
        ],
        IntrinsicId::ProcessRead => &[HostImport(Host::ProcessRead)],
        IntrinsicId::ProcessFollow => &[Helper(Runtime::FollowAddress)],
        IntrinsicId::ProcessScan | IntrinsicId::ModuleScan => &[Helper(Runtime::ScanProcessRange)],
        IntrinsicId::ProcessScanMemory | IntrinsicId::ProcessScanMemoryAny => &[
            Helper(Runtime::ScanProcessRange),
            HostImport(Host::ProcessGetMemoryRangeCount),
            HostImport(Host::ProcessGetMemoryRangeAddress),
            HostImport(Host::ProcessGetMemoryRangeSize),
            HostImport(Host::ProcessGetMemoryRangeFlags),
        ],
        IntrinsicId::ProcessReadRelative32 => &[Helper(Runtime::ReadRelative32)],
        IntrinsicId::ProcessReadUtf8 => &[Helper(Runtime::ReadUtf8String)],
        IntrinsicId::ProcessReadManagedString => &[Helper(Runtime::ReadManagedString)],
        IntrinsicId::ModulePath => &[Helper(Runtime::ModulePath)],
        IntrinsicId::UnityIl2Cpp => &[Helper(Runtime::UnityAttach)],
        IntrinsicId::UnityModuleImage => &[Helper(Runtime::UnityGetImage)],
        IntrinsicId::UnityImageClass => &[Helper(Runtime::UnityGetClass)],
        IntrinsicId::UnityClassField => &[Helper(Runtime::UnityGetFieldOffset)],
        IntrinsicId::UnityClassFieldAny => &[Helper(Runtime::UnityGetFieldAny)],
        IntrinsicId::UnityClassStaticInstance => &[Helper(Runtime::UnityGetStaticInstance)],
        IntrinsicId::GbaAttach => &[Helper(Runtime::GbaAttach)],
        IntrinsicId::GbaEmulatorRead => &[Helper(Runtime::GbaTranslateAddress)],
        IntrinsicId::StringContains
        | IntrinsicId::StringStartsWith
        | IntrinsicId::StringEndsWith
        | IntrinsicId::StringEqualsIgnoreAsciiCase => &[Helper(Runtime::StringMatch)],
        IntrinsicId::StringReplaceAll => &[Helper(Runtime::StringReplaceAll)],
        IntrinsicId::StringSlice => &[Helper(Runtime::StringSlice)],
        IntrinsicId::StringConcat => &[Helper(Runtime::ConcatStrings)],
        IntrinsicId::UnityClassStaticTable => &[HostImport(Host::ProcessRead)],
        IntrinsicId::NextTick
        | IntrinsicId::NumericAdd
        | IntrinsicId::NumericSubtract
        | IntrinsicId::NumericMin
        | IntrinsicId::NumericMax
        | IntrinsicId::FloatAbs
        | IntrinsicId::FloatFloor
        | IntrinsicId::FloatCeil
        | IntrinsicId::FloatRound
        | IntrinsicId::ArrayLength
        | IntrinsicId::ArraySet
        | IntrinsicId::AddressAdd
        | IntrinsicId::ProcessName
        | IntrinsicId::ProcessClosed
        | IntrinsicId::StringLength => &[],
    }
}

const PURE: EffectSet = EffectSet::one(Effect::Pure);
const ALLOCATES: EffectSet = EffectSet::one(Effect::Allocates);
const MUTATES: EffectSet = EffectSet::one(Effect::MutatesValue);
const TIMER_READ: EffectSet = EffectSet::one(Effect::ReadsTimer);
const RUNTIME_READ_ALLOCATES: EffectSet =
    EffectSet::one(Effect::ReadsRuntime).with(Effect::Allocates);
const TIMER_WRITE: EffectSet = EffectSet::one(Effect::WritesTimer);
const RUNTIME_WRITE: EffectSet = EffectSet::one(Effect::WritesRuntime);
const PROCESS: EffectSet =
    EffectSet::one(Effect::ReadsProcess).with(Effect::RequiresAttachedProcess);
const ATTACHED_ALLOCATES: EffectSet =
    EffectSet::one(Effect::RequiresAttachedProcess).with(Effect::Allocates);
const PROCESS_SUSPEND: EffectSet = PROCESS
    .with(Effect::Suspends)
    .with(Effect::CancelsOnProcessClose);
const NEXT_TICK: EffectSet = EffectSet::one(Effect::RequiresAttachedProcess)
    .with(Effect::Suspends)
    .with(Effect::CancelsOnProcessClose);

const NONE: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::None);
const BOOL: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Bool);
const I64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::I64);
const U32: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::U32);
const U64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::U64);
const F64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::F64);
const ADDRESS: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Address);
const STRING: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::String);
const SETTINGS_VIEW: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::SettingsView);
const PROCESS_TYPE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Process);
const SIGNATURE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Signature);
const SIGNATURE_MATCH: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::SignatureMatch);
const MODULE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Module);
const TIMER_STATE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::TimerState);
const INSTANT: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Instant);
const UNITY_MODULE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityModule);
const UNITY_IMAGE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityImage);
const UNITY_CLASS: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityClass);
const UNITY_FIELD: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityField);
const GBA_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::GbaEmulator);
const T: ContractTypeRef = ContractTypeRef::Parameter(0);
const T_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[T],
};
const STRING_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[STRING],
};
const U64_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[U64],
};
const SIGNATURE_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[SIGNATURE],
};
const T_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[T],
};
const ADDRESS_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[ADDRESS],
};
const STRING_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[STRING],
};
const GBA_EMULATOR_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[GBA_EMULATOR],
};

const NO_TYPE_PARAMETERS: Option<&[StdlibCapabilityId]> = None;
const UNCONSTRAINED_T: Option<&[StdlibCapabilityId]> = Some(&[]);
const NUMERIC_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Numeric]);
const FLOAT_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Float]);
const MEMORY_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::MemoryReadable]);
const DISPLAY_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Display]);

const fn value(ty: ContractTypeRef) -> ContractParameter {
    ContractParameter {
        ty,
        rule: ParameterRule::Value,
    }
}

const fn literal(ty: ContractTypeRef, rule: ParameterRule) -> ContractParameter {
    ContractParameter { ty, rule }
}

const fn signature(
    type_parameter_constraints: Option<&'static [StdlibCapabilityId]>,
    receiver: Option<ContractTypeRef>,
    parameters: [Option<ContractParameter>; 3],
    result: ContractTypeRef,
) -> IntrinsicSignature {
    IntrinsicSignature {
        type_parameter_constraints,
        receiver,
        parameters,
        result,
    }
}

macro_rules! params {
    () => {
        [None, None, None]
    };
    ($one:expr $(,)?) => {
        [Some($one), None, None]
    };
    ($one:expr, $two:expr $(,)?) => {
        [Some($one), Some($two), None]
    };
    ($one:expr, $two:expr, $three:expr $(,)?) => {
        [Some($one), Some($two), Some($three)]
    };
}

macro_rules! contract {
    ($id:ident, $shape:ident, $signature:expr, $effects:expr,
     $availability:ident, $lowering:ident) => {
        IntrinsicContract::new(
            IntrinsicId::$id,
            CallableShape::$shape,
            $signature,
            $effects,
            Availability::$availability,
            LoweringClass::$lowering,
        )
    };
}

pub(crate) const fn contract(id: IntrinsicId) -> IntrinsicContract {
    match id {
        IntrinsicId::Print => contract!(
            Print,
            Function,
            signature(DISPLAY_T, None, params![value(T)], NONE),
            RUNTIME_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerSetVariable => contract!(
            TimerSetVariable,
            Function,
            signature(DISPLAY_T, None, params![value(STRING), value(T)], NONE,),
            TIMER_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::RuntimeSetTickRate => contract!(
            RuntimeSetTickRate,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![value(F64)], NONE),
            RUNTIME_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::SettingsEnabled => contract!(
            SettingsEnabled,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(SETTINGS_VIEW),
                params![value(STRING)],
                BOOL,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::InstantNow => contract!(
            InstantNow,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], INSTANT),
            RUNTIME_READ_ALLOCATES,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::NextTick => {
            contract!(
                NextTick,
                Function,
                signature(NO_TYPE_PARAMETERS, None, params![], NONE),
                NEXT_TICK,
                OnAttach,
                Suspension
            )
        }
        IntrinsicId::NumericMin => contract!(
            NumericMin,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::NumericAdd => contract!(
            NumericAdd,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::NumericSubtract => contract!(
            NumericSubtract,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::NumericMax => contract!(
            NumericMax,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::FloatAbs => contract!(
            FloatAbs,
            Method,
            signature(FLOAT_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::FloatFloor => contract!(
            FloatFloor,
            Method,
            signature(FLOAT_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::FloatCeil => contract!(
            FloatCeil,
            Method,
            signature(FLOAT_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::FloatRound => contract!(
            FloatRound,
            Method,
            signature(FLOAT_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArrayLength => contract!(
            ArrayLength,
            Method,
            signature(UNCONSTRAINED_T, Some(T_ARRAY), params![], U32),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArraySet => contract!(
            ArraySet,
            Method,
            signature(
                UNCONSTRAINED_T,
                Some(T_ARRAY),
                params![value(U32), value(T)],
                NONE,
            ),
            MUTATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::AddressAdd => contract!(
            AddressAdd,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(ADDRESS),
                params![value(U64)],
                ADDRESS,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ProcessName => contract!(
            ProcessName,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(PROCESS_TYPE), params![], STRING,),
            ATTACHED_ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ProcessMainModule => contract!(
            ProcessMainModule,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(PROCESS_TYPE), params![], MODULE,),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessClosed => contract!(
            ProcessClosed,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(PROCESS_TYPE), params![], NONE,),
            NEXT_TICK,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessModule => contract!(
            ProcessModule,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![literal(STRING, ParameterRule::StringLiteral)],
                MODULE,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessRead => contract!(
            ProcessRead,
            Method,
            signature(
                MEMORY_T,
                Some(PROCESS_TYPE),
                params![value(ADDRESS)],
                T_RESULT
            ),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::ProcessFollow => contract!(
            ProcessFollow,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(ADDRESS), value(U64_ARRAY)],
                ADDRESS_RESULT,
            ),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::ProcessScan => contract!(
            ProcessScan,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(ADDRESS), value(U64), value(SIGNATURE),],
                ADDRESS,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessScanMemory => contract!(
            ProcessScanMemory,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(SIGNATURE)],
                ADDRESS,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessScanMemoryAny => contract!(
            ProcessScanMemoryAny,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(SIGNATURE_ARRAY)],
                SIGNATURE_MATCH,
            ),
            PROCESS_SUSPEND.with(Effect::Allocates),
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessReadRelative32 => contract!(
            ProcessReadRelative32,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(ADDRESS)],
                ADDRESS_RESULT,
            ),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::ProcessReadUtf8 => contract!(
            ProcessReadUtf8,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(ADDRESS), value(U32)],
                STRING_RESULT,
            ),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::ProcessReadManagedString => contract!(
            ProcessReadManagedString,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(ADDRESS), value(U32)],
                STRING_RESULT,
            ),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::TimerState => contract!(
            TimerState,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], TIMER_STATE),
            TIMER_READ,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerCurrentSplitIndex => contract!(
            TimerCurrentSplitIndex,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], I64),
            TIMER_READ,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerPauseGameTime => contract!(
            TimerPauseGameTime,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], NONE),
            TIMER_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerResumeGameTime => contract!(
            TimerResumeGameTime,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], NONE),
            TIMER_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::UnityIl2Cpp => contract!(
            UnityIl2Cpp,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![value(U32)], UNITY_MODULE,),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::StringLength => contract!(
            StringLength,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(STRING), params![], U32,),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringContains => contract!(
            StringContains,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                BOOL,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringStartsWith => contract!(
            StringStartsWith,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                BOOL,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringEndsWith => contract!(
            StringEndsWith,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                BOOL,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringEqualsIgnoreAsciiCase => contract!(
            StringEqualsIgnoreAsciiCase,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                BOOL,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringReplaceAll => contract!(
            StringReplaceAll,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING), value(STRING)],
                STRING_RESULT,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringSlice => contract!(
            StringSlice,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(U32), value(U32)],
                STRING_RESULT,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringConcat => contract!(
            StringConcat,
            Function,
            signature(
                NO_TYPE_PARAMETERS,
                None,
                params![value(STRING_ARRAY)],
                STRING,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ModuleScan => contract!(
            ModuleScan,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(MODULE),
                params![value(SIGNATURE)],
                ADDRESS,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::ModulePath => contract!(
            ModulePath,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(MODULE), params![], STRING_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::UnityModuleImage => contract!(
            UnityModuleImage,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(UNITY_MODULE),
                params![value(STRING)],
                UNITY_IMAGE,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::UnityImageClass => contract!(
            UnityImageClass,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(UNITY_IMAGE),
                params![value(STRING)],
                UNITY_CLASS,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::UnityClassField => contract!(
            UnityClassField,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(UNITY_CLASS),
                params![value(STRING)],
                U32,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::UnityClassFieldAny => contract!(
            UnityClassFieldAny,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(UNITY_CLASS),
                params![value(STRING_ARRAY)],
                UNITY_FIELD,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::UnityClassStaticTable => contract!(
            UnityClassStaticTable,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(UNITY_CLASS), params![], ADDRESS,),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::UnityClassStaticInstance => contract!(
            UnityClassStaticInstance,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(UNITY_CLASS),
                params![value(STRING_ARRAY)],
                ADDRESS,
            ),
            PROCESS_SUSPEND,
            OnAttach,
            Suspension
        ),
        IntrinsicId::GbaAttach => contract!(
            GbaAttach,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], GBA_EMULATOR_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::GbaEmulatorRead => contract!(
            GbaEmulatorRead,
            Method,
            signature(MEMORY_T, Some(GBA_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::{Parameter, TypeParameter};

    #[test]
    fn every_generated_intrinsic_has_one_self_identifying_contract() {
        for id in IntrinsicId::ALL {
            assert_eq!(contract(*id).id, *id);
        }
    }

    #[test]
    fn backend_dependency_planning_consumes_contract_roots() {
        let planner = include_str!("codegen/dependencies.rs");
        let retired_dispatch = ["match intr", "insic"].concat();

        assert!(!planner.contains(&retired_dispatch));
        assert!(planner.contains("dependency_roots"));
        assert!(
            !contract(IntrinsicId::UnityIl2Cpp)
                .dependency_roots
                .is_empty()
        );
    }

    #[test]
    fn exact_signatures_compare_types_rules_constraints_and_generic_positions() {
        let read = contract(IntrinsicId::ProcessRead);
        let result_of_value = TypeRef::Application {
            constructor: StdlibTypeConstructorId::Result,
            arguments: &[TypeRef::Parameter("Value")],
        };
        let valid = Signature {
            type_parameters: &[TypeParameter {
                name: "Value",
                constraints: &[StdlibCapabilityId::MemoryReadable],
            }],
            explicit_type_parameters: 1,
            parameters: &[Parameter {
                name: "address",
                ty: TypeRef::Core(CoreTypeId::Address),
                rule: ParameterRule::Value,
                documentation: "",
            }],
            result: result_of_value,
        };
        assert!(read.signature.matches(
            ItemKind::Method {
                receiver: TypeRef::Standard(StdlibTypeId::Process),
            },
            valid,
        ));

        let wrong_result = Signature {
            result: TypeRef::Core(CoreTypeId::Address),
            ..valid
        };
        assert!(!read.signature.matches(
            ItemKind::Method {
                receiver: TypeRef::Standard(StdlibTypeId::Process),
            },
            wrong_result,
        ));

        let module = contract(IntrinsicId::ProcessModule);
        let wrong_rule = Signature {
            type_parameters: &[],
            explicit_type_parameters: 0,
            parameters: &[Parameter {
                name: "name",
                ty: TypeRef::Standard(StdlibTypeId::String),
                rule: ParameterRule::Value,
                documentation: "",
            }],
            result: TypeRef::Standard(StdlibTypeId::Module),
        };
        assert!(!module.signature.matches(ItemKind::Function, wrong_rule));
    }
}
