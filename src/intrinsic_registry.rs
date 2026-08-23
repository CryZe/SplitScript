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
/// Native UTF-16 reads share the 4096-byte bounded-input policy.
pub(crate) const MAX_NATIVE_UTF16_UNITS: u32 = MAX_NATIVE_STRING_BYTES / 2;

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
    FormatChar,
    QuoteDebugString,
    StringEquality,
    StringMatch,
    StringFind,
    StringRFind,
    StringAsciiCase,
    StringReplaceAll,
    StringSplit,
    StringParseInteger,
    DecimalLeftShift,
    DecimalRightShift,
    DecimalRound,
    StringParseFloat,
    StringInspect,
    StringSlice,
    StringTrimAsciiWhitespace,
    StringPad,
    ScanProcessRange,
    ReadRelative32,
    ScanRelative32TargetRange,
    StringFromMemory,
    Utf16StringFromMemory,
    ReadUtf8String,
    ReadUtf16LeString,
    ReadManagedString,
    LoadedModule,
    ModulePath,
    ProcessPath,
    RuntimeOperatingSystem,
    RuntimeArchitecture,
    CStringEquality,
    BackingFieldEquality,
    UnityGetImage,
    UnityGetClass,
    UnityGetFieldOffset,
    UnityGetFieldAny,
    UnityGetStaticInstance,
    JoinStrings,
    IndentDisplay,
    WrapDebugEntry,
    WrapDebugVariant,
    FollowAddress,
    GBATranslateAddress,
    GBAReadMemory,
    GCNTranslateAddress,
    GCNReadMemory,
    WiiTranslateAddress,
    WiiReadMemory,
    Ps2TranslateAddress,
    Ps2ReadMemory,
    Ps1TranslateAddress,
    Ps1ReadMemory,
    SmsTranslateAddress,
    SmsReadMemory,
    GenesisReadMemory,
    RefreshSettings,
    SettingsEnabled,
    SettingsContains,
}

/// Backend information shared by every emulator-style state provider read.
///
/// Keeping this next to the intrinsic contracts makes the provider declaration
/// the only public source of truth while both ordinary method calls and state
/// polling consume the same lowering metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderReadContract {
    pub reader: RuntimeHelperId,
    pub byte_order: ProviderByteOrder,
    pub invalid_address: &'static str,
    pub read_failure: &'static str,
}

/// The byte order of scalar values exposed by an emulator provider.
///
/// This belongs to the provider read contract rather than `MemoryReadable`:
/// the latter describes a value's fixed shape, while the provider describes
/// how bytes from the emulated machine are encoded. A future provider whose
/// byte order varies by backend can extend this with provider-owned runtime
/// state without duplicating record or array layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderByteOrder {
    Little,
    Big,
}

pub(crate) const fn provider_read_contract(intrinsic: IntrinsicId) -> Option<ProviderReadContract> {
    match intrinsic {
        IntrinsicId::GBAEmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::GBAReadMemory,
            byte_order: ProviderByteOrder::Little,
            invalid_address: "invalid or unavailable GBA memory address",
            read_failure: "GBA memory read failed",
        }),
        IntrinsicId::Ps2EmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::Ps2ReadMemory,
            byte_order: ProviderByteOrder::Little,
            invalid_address: "invalid or unavailable PS2 memory address",
            read_failure: "PS2 memory read failed",
        }),
        IntrinsicId::Ps1EmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::Ps1ReadMemory,
            byte_order: ProviderByteOrder::Little,
            invalid_address: "invalid or unavailable PS1 memory address",
            read_failure: "PS1 memory read failed",
        }),
        IntrinsicId::SmsEmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::SmsReadMemory,
            byte_order: ProviderByteOrder::Little,
            invalid_address: "invalid or unavailable SMS memory address",
            read_failure: "SMS memory read failed",
        }),
        IntrinsicId::GenesisEmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::GenesisReadMemory,
            byte_order: ProviderByteOrder::Big,
            invalid_address: "invalid or unavailable Genesis memory address",
            read_failure: "Genesis memory read failed",
        }),
        IntrinsicId::GCNEmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::GCNReadMemory,
            byte_order: ProviderByteOrder::Big,
            invalid_address: "invalid or unavailable GameCube memory address",
            read_failure: "GameCube memory read failed",
        }),
        IntrinsicId::WiiEmulatorRead => Some(ProviderReadContract {
            reader: RuntimeHelperId::WiiReadMemory,
            byte_order: ProviderByteOrder::Big,
            invalid_address: "invalid or unavailable Wii memory address",
            read_failure: "Wii memory read failed",
        }),
        _ => None,
    }
}

impl RuntimeHelperId {
    pub(crate) const COUNT: usize = Self::SettingsContains as usize + 1;

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
    Standard(StdlibTypeId),
    Expression,
    ResultValue,
    Receiver,
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
        IntrinsicId::ProcessFindMemoryRange => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ProcessScan => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ModuleScanRelative32Target => scratch(ScratchType::Core(CoreTypeId::U64), 7),
        IntrinsicId::ProcessScanMemory => scratch(ScratchType::Core(CoreTypeId::U64), 7),
        IntrinsicId::ProcessScanMemoryAny => scratch(ScratchType::Core(CoreTypeId::U64), 8),
        IntrinsicId::ModuleScan => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ModuleScanAny => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ProcessFollow
        | IntrinsicId::ProcessReadRelative32
        | IntrinsicId::UnityClassField
        | IntrinsicId::UnityClassStaticInstance
        | IntrinsicId::UnityClassStaticTable => scratch(ScratchType::Core(CoreTypeId::U64), 1),
        IntrinsicId::UnityModuleImage
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
        IntrinsicId::ProcessScan
        | IntrinsicId::ModuleScanRelative32Target
        | IntrinsicId::ModuleScan => scratch(ScratchType::Core(CoreTypeId::U64), 2),
        IntrinsicId::ProcessScanMemory => scratch(ScratchType::Core(CoreTypeId::U64), 3),
        IntrinsicId::ProcessScanMemoryAny => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        IntrinsicId::ModuleScanAny => scratch(ScratchType::Core(CoreTypeId::U64), 4),
        IntrinsicId::ProcessFindMemoryRange => scratch(ScratchType::Core(CoreTypeId::U64), 5),
        _ => None,
    }
}

const fn synchronous_scratch(id: IntrinsicId) -> Option<ScratchPolicy> {
    match id {
        IntrinsicId::ArrayIterator
        | IntrinsicId::ArrayIteratorNext
        | IntrinsicId::SetIterator
        | IntrinsicId::SetIteratorNext
        | IntrinsicId::ExclusiveRangeIterator
        | IntrinsicId::ExclusiveRangeIteratorNext
        | IntrinsicId::InclusiveRangeIterator
        | IntrinsicId::InclusiveRangeIteratorNext => scratch(ScratchType::Receiver, 1),
        IntrinsicId::NumericSwapBytes => scratch(ScratchType::Expression, 1),
        IntrinsicId::ProcessLoadedModule => scratch(ScratchType::Standard(StdlibTypeId::Module), 1),
        IntrinsicId::ProcessMemoryRanges => scratch(ScratchType::Expression, 1),
        IntrinsicId::NumericMin | IntrinsicId::NumericMax => scratch(ScratchType::Expression, 2),
        IntrinsicId::TimerState => scratch(ScratchType::Core(CoreTypeId::U32), 1),
        IntrinsicId::TimerCurrentSplitIndex => scratch(ScratchType::Core(CoreTypeId::I64), 1),
        IntrinsicId::TimerSegmentWasSplit => scratch(ScratchType::Core(CoreTypeId::I32), 1),
        IntrinsicId::StringIndexOf | IntrinsicId::StringLastIndexOf => {
            scratch(ScratchType::Core(CoreTypeId::I32), 1)
        }
        IntrinsicId::ProcessFollow
        | IntrinsicId::ProcessReadRelative32
        | IntrinsicId::ProcessReadUtf8
        | IntrinsicId::ProcessReadUtf16Le
        | IntrinsicId::ProcessReadManagedString
        | IntrinsicId::ModulePath
        | IntrinsicId::ProcessPath
        | IntrinsicId::RuntimeOperatingSystem
        | IntrinsicId::RuntimeArchitecture
        | IntrinsicId::StringReplaceAll
        | IntrinsicId::StringSplit
        | IntrinsicId::StringParse
        | IntrinsicId::IntegerToStringRadix
        | IntrinsicId::StringByteAt
        | IntrinsicId::StringCharAt
        | IntrinsicId::StringSlice => scratch(ScratchType::ResultValue, 1),
        IntrinsicId::GBAEmulatorRead
        | IntrinsicId::GCNEmulatorRead
        | IntrinsicId::WiiEmulatorRead
        | IntrinsicId::Ps2EmulatorRead
        | IntrinsicId::Ps1EmulatorRead
        | IntrinsicId::SmsEmulatorRead
        | IntrinsicId::GenesisEmulatorRead => scratch(ScratchType::Core(CoreTypeId::Address), 1),
        _ => None,
    }
}

const fn dependency_roots(id: IntrinsicId) -> &'static [DependencyRoot] {
    use AbiImportId as Host;
    use DependencyRoot::{Helper, HostImport};
    use RuntimeHelperId as Runtime;

    match id {
        IntrinsicId::Print => &[
            Helper(Runtime::PrintString),
            Helper(Runtime::FormatI64),
            Helper(Runtime::FormatChar),
        ],
        IntrinsicId::TimerSetVariable => &[
            Helper(Runtime::TimerSetVariable),
            Helper(Runtime::FormatI64),
            Helper(Runtime::FormatChar),
        ],
        IntrinsicId::IntegerToStringRadix => &[Helper(Runtime::FormatI64)],
        IntrinsicId::RuntimeSetTickRate => &[HostImport(Host::RuntimeSetTickRate)],
        IntrinsicId::SettingsEnabled => &[Helper(Runtime::SettingsEnabled)],
        IntrinsicId::SettingsContains => &[Helper(Runtime::SettingsContains)],
        IntrinsicId::InstantNow => &[HostImport(Host::WasiClockTimeGet)],
        IntrinsicId::TimerState => &[HostImport(Host::TimerGetState)],
        IntrinsicId::TimerCurrentSplitIndex => &[HostImport(Host::TimerCurrentSplitIndex)],
        IntrinsicId::TimerSegmentWasSplit => &[HostImport(Host::TimerSegmentWasSplit)],
        IntrinsicId::TimerSkipSplit => &[HostImport(Host::TimerSkipSplit)],
        IntrinsicId::TimerUndoSplit => &[HostImport(Host::TimerUndoSplit)],
        IntrinsicId::TimerPauseGameTime => &[HostImport(Host::TimerPauseGameTime)],
        IntrinsicId::TimerResumeGameTime => &[HostImport(Host::TimerResumeGameTime)],
        IntrinsicId::ProcessMainModule | IntrinsicId::ProcessModule => &[
            HostImport(Host::ProcessGetModuleAddress),
            HostImport(Host::ProcessGetModuleSize),
        ],
        IntrinsicId::ProcessLoadedModule => &[Helper(Runtime::LoadedModule)],
        IntrinsicId::ProcessFindMemoryRange | IntrinsicId::ProcessMemoryRanges => &[
            HostImport(Host::ProcessGetMemoryRangeCount),
            HostImport(Host::ProcessGetMemoryRangeAddress),
            HostImport(Host::ProcessGetMemoryRangeSize),
            HostImport(Host::ProcessGetMemoryRangeFlags),
        ],
        IntrinsicId::ProcessRead => &[HostImport(Host::ProcessRead)],
        IntrinsicId::ProcessFollow => &[Helper(Runtime::FollowAddress)],
        IntrinsicId::ProcessScan | IntrinsicId::ModuleScan | IntrinsicId::ModuleScanAny => {
            &[Helper(Runtime::ScanProcessRange)]
        }
        IntrinsicId::ModuleScanRelative32Target => &[Helper(Runtime::ScanRelative32TargetRange)],
        IntrinsicId::ProcessScanMemory | IntrinsicId::ProcessScanMemoryAny => &[
            Helper(Runtime::ScanProcessRange),
            HostImport(Host::ProcessGetMemoryRangeCount),
            HostImport(Host::ProcessGetMemoryRangeAddress),
            HostImport(Host::ProcessGetMemoryRangeSize),
            HostImport(Host::ProcessGetMemoryRangeFlags),
        ],
        IntrinsicId::ProcessReadRelative32 => &[Helper(Runtime::ReadRelative32)],
        IntrinsicId::ProcessReadUtf8 => &[Helper(Runtime::ReadUtf8String)],
        IntrinsicId::ProcessReadUtf16Le => &[Helper(Runtime::ReadUtf16LeString)],
        IntrinsicId::ProcessReadManagedString => &[Helper(Runtime::ReadManagedString)],
        IntrinsicId::ModulePath => &[Helper(Runtime::ModulePath)],
        IntrinsicId::ProcessPath => &[Helper(Runtime::ProcessPath)],
        IntrinsicId::RuntimeOperatingSystem => &[Helper(Runtime::RuntimeOperatingSystem)],
        IntrinsicId::RuntimeArchitecture => &[Helper(Runtime::RuntimeArchitecture)],
        IntrinsicId::UnityModuleImage => &[Helper(Runtime::UnityGetImage)],
        IntrinsicId::UnityImageClass => &[Helper(Runtime::UnityGetClass)],
        IntrinsicId::UnityClassField => &[Helper(Runtime::UnityGetFieldOffset)],
        IntrinsicId::UnityClassFieldAny => &[Helper(Runtime::UnityGetFieldAny)],
        IntrinsicId::UnityClassStaticInstance => &[Helper(Runtime::UnityGetStaticInstance)],
        IntrinsicId::GBAEmulatorRead => &[Helper(Runtime::GBAReadMemory)],
        IntrinsicId::GCNEmulatorRead => &[Helper(Runtime::GCNReadMemory)],
        IntrinsicId::WiiEmulatorRead => &[Helper(Runtime::WiiReadMemory)],
        IntrinsicId::Ps2EmulatorRead => &[Helper(Runtime::Ps2ReadMemory)],
        IntrinsicId::Ps1EmulatorRead => &[Helper(Runtime::Ps1ReadMemory)],
        IntrinsicId::SmsEmulatorRead => &[Helper(Runtime::SmsReadMemory)],
        IntrinsicId::GenesisEmulatorRead => &[Helper(Runtime::GenesisReadMemory)],
        IntrinsicId::StringContains
        | IntrinsicId::StringStartsWith
        | IntrinsicId::StringEndsWith
        | IntrinsicId::StringEqualsIgnoreAsciiCase => &[Helper(Runtime::StringMatch)],
        IntrinsicId::StringIndexOf => &[Helper(Runtime::StringFind)],
        IntrinsicId::StringLastIndexOf => &[Helper(Runtime::StringRFind)],
        IntrinsicId::StringToAsciiLowerCase | IntrinsicId::StringToAsciiUpperCase => {
            &[Helper(Runtime::StringAsciiCase)]
        }
        IntrinsicId::StringReplaceAll => &[Helper(Runtime::StringReplaceAll)],
        IntrinsicId::StringSplit => &[Helper(Runtime::StringSplit)],
        IntrinsicId::StringParse => &[
            Helper(Runtime::StringParseInteger),
            Helper(Runtime::StringParseFloat),
        ],
        IntrinsicId::StringByteAt | IntrinsicId::StringCharAt => &[Helper(Runtime::StringInspect)],
        IntrinsicId::StringSlice => &[Helper(Runtime::StringSlice)],
        IntrinsicId::StringTrimAsciiWhitespace => &[Helper(Runtime::StringTrimAsciiWhitespace)],
        IntrinsicId::StringPadStart | IntrinsicId::StringPadEnd => &[Helper(Runtime::StringPad)],
        IntrinsicId::StringConcat | IntrinsicId::StringJoin => &[Helper(Runtime::JoinStrings)],
        IntrinsicId::UnityClassStaticTable => &[HostImport(Host::ProcessRead)],
        IntrinsicId::NextTick
        | IntrinsicId::BoolNot
        | IntrinsicId::IntegerBitNot
        | IntrinsicId::NumericSwapBytes
        | IntrinsicId::NumericAdd
        | IntrinsicId::NumericSubtract
        | IntrinsicId::NumericMultiply
        | IntrinsicId::NumericDivide
        | IntrinsicId::IntegerRemainder
        | IntrinsicId::IntegerBitOr
        | IntrinsicId::IntegerBitXor
        | IntrinsicId::IntegerBitAnd
        | IntrinsicId::IntegerShiftLeft
        | IntrinsicId::IntegerShiftRight
        | IntrinsicId::EquatableEquals
        | IntrinsicId::EquatableNotEquals
        | IntrinsicId::SignedNegate
        | IntrinsicId::NumericMin
        | IntrinsicId::NumericMax
        | IntrinsicId::FloatSqrt
        | IntrinsicId::FloatTruncate
        | IntrinsicId::FloatFloor
        | IntrinsicId::FloatCeil
        | IntrinsicId::FloatRound
        | IntrinsicId::F32FromBits
        | IntrinsicId::F32ToBits
        | IntrinsicId::F64FromBits
        | IntrinsicId::F64ToBits
        | IntrinsicId::ArrayLength
        | IntrinsicId::ArraySet
        | IntrinsicId::ArrayPush
        | IntrinsicId::ArrayRemoveAt
        | IntrinsicId::ArrayClear
        | IntrinsicId::ArrayIterator
        | IntrinsicId::ArrayIteratorNext
        | IntrinsicId::SetIterator
        | IntrinsicId::SetIteratorNext
        | IntrinsicId::ExclusiveRangeIterator
        | IntrinsicId::ExclusiveRangeIteratorNext
        | IntrinsicId::InclusiveRangeIterator
        | IntrinsicId::InclusiveRangeIteratorNext
        | IntrinsicId::SetNew
        | IntrinsicId::SetLength
        | IntrinsicId::SetContains
        | IntrinsicId::SetInsert
        | IntrinsicId::SetRemove
        | IntrinsicId::SetClear
        | IntrinsicId::AddressAdd
        | IntrinsicId::ProcessName
        | IntrinsicId::ProcessClosed
        | IntrinsicId::StringLength => &[],
    }
}

const PURE: EffectSet = EffectSet::one(Effect::Pure);
const ALLOCATES: EffectSet = EffectSet::one(Effect::Allocates);
const MUTATES: EffectSet = EffectSet::one(Effect::MutatesValue);
const MUTATES_ALLOCATES: EffectSet = MUTATES.with(Effect::Allocates);
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
const NEVER: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Never);
const BOOL: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Bool);
const CHAR: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Char);
const U8: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::U8);
const I64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::I64);
const U32: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::U32);
const U64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::U64);
const F32: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::F32);
const F64: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::F64);
const ADDRESS: ContractTypeRef = ContractTypeRef::Core(CoreTypeId::Address);
const STRING: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::String);
const SETTINGS_VIEW: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::SettingsView);
const PROCESS_TYPE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Process);
const SIGNATURE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Signature);
const SIGNATURE_MATCH: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::SignatureMatch);
const MODULE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Module);
const MODULE_OPTION: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: &[MODULE],
};
const MEMORY_RANGE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::MemoryRange);
const MEMORY_RANGE_ACCESS: ContractTypeRef =
    ContractTypeRef::Standard(StdlibTypeId::MemoryRangeAccess);
const TIMER_STATE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::TimerState);
const INSTANT: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::Instant);
const UNITY_MODULE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityModule);
const UNITY_IMAGE: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityImage);
const UNITY_CLASS: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityClass);
const UNITY_FIELD: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::UnityField);
const GBA_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::GBAEmulator);
const GCN_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::GCNEmulator);
const WII_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::WiiEmulator);
const PS2_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::PS2Emulator);
const PS1_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::PS1Emulator);
const SMS_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::SMSEmulator);
const GENESIS_EMULATOR: ContractTypeRef = ContractTypeRef::Standard(StdlibTypeId::GenesisEmulator);
const T: ContractTypeRef = ContractTypeRef::Parameter(0);
const T_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[T],
};
const T_SET: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Set,
    arguments: &[T],
};
const T_ARRAY_ITERATOR: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::ArrayIterator,
    arguments: &[T],
};
const T_SET_ITERATOR: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::SetIterator,
    arguments: &[T],
};
const T_EXCLUSIVE_RANGE: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::ExclusiveRange,
    arguments: &[T],
};
const T_EXCLUSIVE_RANGE_ITERATOR: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::ExclusiveRangeIterator,
    arguments: &[T],
};
const T_INCLUSIVE_RANGE: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::InclusiveRange,
    arguments: &[T],
};
const T_INCLUSIVE_RANGE_ITERATOR: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::InclusiveRangeIterator,
    arguments: &[T],
};
const T_ITERATOR_STEP: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::IteratorStep,
    arguments: &[T],
};
const STRING_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[STRING],
};
const I64_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[I64],
};
const U64_OPTION: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: &[U64],
};
const U32_OPTION: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: &[U32],
};
const BOOL_OPTION: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: &[BOOL],
};
const MEMORY_RANGE_OPTION: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: &[MEMORY_RANGE],
};
const MEMORY_RANGE_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[MEMORY_RANGE],
};
const SIGNATURE_ARRAY: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: &[SIGNATURE],
};
const T_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[T],
};
const U8_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[U8],
};
const CHAR_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[CHAR],
};
const ADDRESS_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[ADDRESS],
};
const STRING_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[STRING],
};
const STRING_ARRAY_RESULT: ContractTypeRef = ContractTypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: &[STRING_ARRAY],
};

const NO_TYPE_PARAMETERS: Option<&[StdlibCapabilityId]> = None;
const UNCONSTRAINED_T: Option<&[StdlibCapabilityId]> = Some(&[]);
const NUMERIC_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Numeric]);
const INTEGER_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Integer]);
const SIGNED_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Signed]);
const FLOAT_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Float]);
const EQUATABLE_T: Option<&[StdlibCapabilityId]> = Some(&[StdlibCapabilityId::Equatable]);
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
            signature(NUMERIC_T, None, params![value(T)], NONE),
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
        IntrinsicId::SettingsContains => contract!(
            SettingsContains,
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
        IntrinsicId::BoolNot => contract!(
            BoolNot,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(BOOL), params![], BOOL),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerBitNot => contract!(
            IntegerBitNot,
            Method,
            signature(INTEGER_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::NumericSwapBytes => contract!(
            NumericSwapBytes,
            Method,
            signature(NUMERIC_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerToStringRadix => contract!(
            IntegerToStringRadix,
            Method,
            signature(INTEGER_T, Some(T), params![value(U32)], STRING_RESULT),
            ALLOCATES,
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
        IntrinsicId::NumericMultiply => contract!(
            NumericMultiply,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::NumericDivide => contract!(
            NumericDivide,
            Method,
            signature(NUMERIC_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerRemainder => contract!(
            IntegerRemainder,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerBitOr => contract!(
            IntegerBitOr,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerBitXor => contract!(
            IntegerBitXor,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerBitAnd => contract!(
            IntegerBitAnd,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerShiftLeft => contract!(
            IntegerShiftLeft,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::IntegerShiftRight => contract!(
            IntegerShiftRight,
            Method,
            signature(INTEGER_T, Some(T), params![value(T)], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::EquatableEquals => contract!(
            EquatableEquals,
            Method,
            signature(EQUATABLE_T, Some(T), params![value(T)], BOOL),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::EquatableNotEquals => contract!(
            EquatableNotEquals,
            Method,
            signature(EQUATABLE_T, Some(T), params![value(T)], BOOL),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SignedNegate => contract!(
            SignedNegate,
            Method,
            signature(SIGNED_T, Some(T), params![], T),
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
        IntrinsicId::FloatSqrt => contract!(
            FloatSqrt,
            Method,
            signature(FLOAT_T, Some(T), params![], T),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::FloatTruncate => contract!(
            FloatTruncate,
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
        IntrinsicId::F32FromBits => contract!(
            F32FromBits,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![value(U32)], F32),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::F32ToBits => contract!(
            F32ToBits,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(F32), params![], U32),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::F64FromBits => contract!(
            F64FromBits,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![value(U64)], F64),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::F64ToBits => contract!(
            F64ToBits,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(F64), params![], U64),
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
        IntrinsicId::ArrayPush => contract!(
            ArrayPush,
            Method,
            signature(UNCONSTRAINED_T, Some(T_ARRAY), params![value(T)], NONE),
            MUTATES_ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArrayRemoveAt => contract!(
            ArrayRemoveAt,
            Method,
            signature(UNCONSTRAINED_T, Some(T_ARRAY), params![value(U32)], NONE),
            MUTATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArrayClear => contract!(
            ArrayClear,
            Method,
            signature(UNCONSTRAINED_T, Some(T_ARRAY), params![], NONE),
            MUTATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArrayIterator => contract!(
            ArrayIterator,
            Method,
            signature(UNCONSTRAINED_T, Some(T_ARRAY), params![], T_ARRAY_ITERATOR),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ArrayIteratorNext => contract!(
            ArrayIteratorNext,
            Method,
            signature(
                UNCONSTRAINED_T,
                Some(T_ARRAY_ITERATOR),
                params![],
                T_ITERATOR_STEP,
            ),
            MUTATES_ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetNew => contract!(
            SetNew,
            Function,
            signature(EQUATABLE_T, None, params![], T_SET),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetLength => contract!(
            SetLength,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![], U32),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetContains => contract!(
            SetContains,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![value(T)], BOOL),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetInsert => contract!(
            SetInsert,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![value(T)], BOOL),
            ALLOCATES.with(Effect::MutatesValue),
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetRemove => contract!(
            SetRemove,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![value(T)], BOOL),
            MUTATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetClear => contract!(
            SetClear,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![], NONE),
            ALLOCATES.with(Effect::MutatesValue),
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetIterator => contract!(
            SetIterator,
            Method,
            signature(EQUATABLE_T, Some(T_SET), params![], T_SET_ITERATOR),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::SetIteratorNext => contract!(
            SetIteratorNext,
            Method,
            signature(
                EQUATABLE_T,
                Some(T_SET_ITERATOR),
                params![],
                T_ITERATOR_STEP,
            ),
            MUTATES_ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ExclusiveRangeIterator => contract!(
            ExclusiveRangeIterator,
            Method,
            signature(
                INTEGER_T,
                Some(T_EXCLUSIVE_RANGE),
                params![],
                T_EXCLUSIVE_RANGE_ITERATOR,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::ExclusiveRangeIteratorNext => contract!(
            ExclusiveRangeIteratorNext,
            Method,
            signature(
                INTEGER_T,
                Some(T_EXCLUSIVE_RANGE_ITERATOR),
                params![],
                T_ITERATOR_STEP,
            ),
            MUTATES_ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::InclusiveRangeIterator => contract!(
            InclusiveRangeIterator,
            Method,
            signature(
                INTEGER_T,
                Some(T_INCLUSIVE_RANGE),
                params![],
                T_INCLUSIVE_RANGE_ITERATOR,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::InclusiveRangeIteratorNext => contract!(
            InclusiveRangeIteratorNext,
            Method,
            signature(
                INTEGER_T,
                Some(T_INCLUSIVE_RANGE_ITERATOR),
                params![],
                T_ITERATOR_STEP,
            ),
            MUTATES_ALLOCATES,
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
        IntrinsicId::ProcessPath => contract!(
            ProcessPath,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![],
                STRING_RESULT,
            ),
            PROCESS.with(Effect::Allocates),
            Everywhere,
            Retryable
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
            signature(NO_TYPE_PARAMETERS, Some(PROCESS_TYPE), params![], NEVER,),
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
        IntrinsicId::ProcessLoadedModule => contract!(
            ProcessLoadedModule,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(STRING)],
                MODULE_OPTION,
            ),
            PROCESS.with(Effect::Allocates),
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::ProcessFindMemoryRange => contract!(
            ProcessFindMemoryRange,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![value(U64), value(MEMORY_RANGE_ACCESS)],
                MEMORY_RANGE_OPTION,
            ),
            PROCESS_SUSPEND.with(Effect::Allocates),
            OnAttach,
            Suspension
        ),
        IntrinsicId::ProcessMemoryRanges => contract!(
            ProcessMemoryRanges,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(PROCESS_TYPE),
                params![],
                MEMORY_RANGE_ARRAY,
            ),
            PROCESS.with(Effect::Allocates),
            Everywhere,
            HostBoundary
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
                params![value(ADDRESS), value(I64_ARRAY)],
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
        IntrinsicId::ModuleScanRelative32Target => contract!(
            ModuleScanRelative32Target,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(MODULE),
                params![value(SIGNATURE), value(U64), value(ADDRESS)],
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
        IntrinsicId::ModuleScanAny => contract!(
            ModuleScanAny,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(MODULE),
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
        IntrinsicId::ProcessReadUtf16Le => contract!(
            ProcessReadUtf16Le,
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
            signature(NO_TYPE_PARAMETERS, None, params![], U64_OPTION),
            TIMER_READ.with(Effect::Allocates),
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerSegmentWasSplit => contract!(
            TimerSegmentWasSplit,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![value(U64)], BOOL_OPTION),
            TIMER_READ.with(Effect::Allocates),
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerSkipSplit => contract!(
            TimerSkipSplit,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], NONE),
            TIMER_WRITE,
            Everywhere,
            HostBoundary
        ),
        IntrinsicId::TimerUndoSplit => contract!(
            TimerUndoSplit,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], NONE),
            TIMER_WRITE,
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
        IntrinsicId::StringIndexOf => contract!(
            StringIndexOf,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                U32_OPTION,
            ),
            PURE,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringLastIndexOf => contract!(
            StringLastIndexOf,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                U32_OPTION,
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
        IntrinsicId::StringToAsciiLowerCase => contract!(
            StringToAsciiLowerCase,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(STRING), params![], STRING),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringToAsciiUpperCase => contract!(
            StringToAsciiUpperCase,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(STRING), params![], STRING),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringTrimAsciiWhitespace => contract!(
            StringTrimAsciiWhitespace,
            Method,
            signature(NO_TYPE_PARAMETERS, Some(STRING), params![], STRING),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringPadStart => contract!(
            StringPadStart,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(U32), value(CHAR)],
                STRING,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringPadEnd => contract!(
            StringPadEnd,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(U32), value(CHAR)],
                STRING,
            ),
            ALLOCATES,
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
        IntrinsicId::StringSplit => contract!(
            StringSplit,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(STRING)],
                STRING_ARRAY_RESULT,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringParse => contract!(
            StringParse,
            Method,
            signature(NUMERIC_T, Some(STRING), params![], T_RESULT),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringByteAt => contract!(
            StringByteAt,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(U32)],
                U8_RESULT,
            ),
            ALLOCATES,
            Everywhere,
            RepresentationPrimitive
        ),
        IntrinsicId::StringCharAt => contract!(
            StringCharAt,
            Method,
            signature(
                NO_TYPE_PARAMETERS,
                Some(STRING),
                params![value(U32)],
                CHAR_RESULT,
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
        IntrinsicId::StringJoin => contract!(
            StringJoin,
            Function,
            signature(
                NO_TYPE_PARAMETERS,
                None,
                params![value(STRING_ARRAY), value(STRING)],
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
        IntrinsicId::RuntimeOperatingSystem => contract!(
            RuntimeOperatingSystem,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], STRING_RESULT,),
            RUNTIME_READ_ALLOCATES,
            Everywhere,
            Retryable
        ),
        IntrinsicId::RuntimeArchitecture => contract!(
            RuntimeArchitecture,
            Function,
            signature(NO_TYPE_PARAMETERS, None, params![], STRING_RESULT,),
            RUNTIME_READ_ALLOCATES,
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
        IntrinsicId::GBAEmulatorRead => contract!(
            GBAEmulatorRead,
            Method,
            signature(MEMORY_T, Some(GBA_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::GCNEmulatorRead => contract!(
            GCNEmulatorRead,
            Method,
            signature(MEMORY_T, Some(GCN_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::WiiEmulatorRead => contract!(
            WiiEmulatorRead,
            Method,
            signature(MEMORY_T, Some(WII_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::Ps2EmulatorRead => contract!(
            Ps2EmulatorRead,
            Method,
            signature(MEMORY_T, Some(PS2_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::Ps1EmulatorRead => contract!(
            Ps1EmulatorRead,
            Method,
            signature(MEMORY_T, Some(PS1_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::SmsEmulatorRead => contract!(
            SmsEmulatorRead,
            Method,
            signature(MEMORY_T, Some(SMS_EMULATOR), params![value(U32)], T_RESULT,),
            PROCESS,
            Everywhere,
            Retryable
        ),
        IntrinsicId::GenesisEmulatorRead => contract!(
            GenesisEmulatorRead,
            Method,
            signature(
                MEMORY_T,
                Some(GENESIS_EMULATOR),
                params![value(U32)],
                T_RESULT,
            ),
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
            !contract(IntrinsicId::ProcessScan)
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
            result_is_async: false,
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
            result_is_async: false,
            result: TypeRef::Standard(StdlibTypeId::Module),
        };
        assert!(!module.signature.matches(ItemKind::Function, wrong_rule));
    }
}
