//! Declarative standard-library surface shared by compiler and tooling.
//!
//! Source names, callable shapes, type schemes, effects, and documentation live
//! here. Type checking resolves calls to stable item IDs. Backends only receive
//! stable intrinsic IDs and concrete inferred type arguments.

mod declarations;

pub use declarations::{
    CoreType, CoreTypeId, DeclaredTypeRef, FieldVisibility, RuntimeRepresentation,
    ScalarMemoryLayout, StdlibCapabilityId, StdlibField, StdlibFieldId, StdlibNamespace,
    StdlibNamespaceId, StdlibOwner, StdlibSymbolId, StdlibType, StdlibTypeConstructorId,
    StdlibTypeId, StdlibTypeKind, StdlibVariant, StdlibVariantId, ValueUsage,
};

use declarations::{CORE_TYPES, FIELDS, NAMESPACES, TYPES, VARIANTS};

use std::collections::HashSet;

use crate::{
    catalog::{Documentation, Example},
    types::{BuiltinType, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibItemId {
    NumericMin,
    NumericMax,
    NumericClamp,
    Print,
    SetVariable,
    SetTickRate,
    NextTick,
    StringLength,
    StringConcat,
    TimerState,
    ProcessModule,
    ProcessRead,
    ProcessFollow,
    ProcessScan,
    ProcessReadRelative32,
    ProcessReadManagedString,
    UnityIl2Cpp,
    DurationFromFrames,
    DurationFromParts,
    DurationFromSeconds,
    AddressOffset,
    AddressAdd,
    ModuleScan,
    UnityModuleImage,
    UnityImageClass,
    UnityClassField,
    UnityClassFieldAny,
    UnityClassStaticTable,
    UnityClassStaticInstance,
    ArrayLength,
    ArrayGet,
    ArraySet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicId {
    NumericMin,
    NumericMax,
    NumericClamp,
    Print,
    StringLength,
    StringConcat,
    TimerSetVariable,
    TimerState,
    RuntimeSetTickRate,
    NextTick,
    ProcessModule,
    ProcessRead,
    ProcessFollow,
    ProcessScan,
    ProcessReadRelative32,
    ProcessReadManagedString,
    UnityIl2Cpp,
    DurationFromFrames,
    DurationFromParts,
    DurationSaturatingSecondsF32,
    AddressOffset,
    AddressAdd,
    ModuleScan,
    UnityModuleImage,
    UnityImageClass,
    UnityClassField,
    UnityClassFieldAny,
    UnityClassStaticTable,
    UnityClassStaticInstance,
    ArrayLength,
    ArrayGet,
    ArraySet,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeConstraint {
    Numeric,
    MemoryReadable,
}

impl TypeConstraint {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Numeric => "Numeric",
            Self::MemoryReadable => "MemoryReadable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
    Variable(&'static str),
    Array(&'static TypeRef),
    Result(&'static TypeRef),
}

impl TypeRef {
    fn render(self) -> String {
        self.render_with(&[])
    }

    fn render_with(self, substitutions: &[(&str, String)]) -> String {
        match self {
            Self::Core(ty) => ty.to_string(),
            Self::Standard(ty) => StandardLibrary::new().type_decl(ty).name.to_owned(),
            Self::Variable(name) => substitutions
                .iter()
                .find_map(|(parameter, ty)| (*parameter == name).then(|| ty.clone()))
                .unwrap_or_else(|| name.to_owned()),
            Self::Array(element) => format!("[{}]", element.render_with(substitutions)),
            Self::Result(value) => format!("{}!", value.render_with(substitutions)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeParameter {
    pub name: &'static str,
    pub constraints: &'static [TypeConstraint],
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
    pub effects: &'static [Effect],
    pub availability: Availability,
    pub deprecation: Option<Deprecation>,
    pub documentation: Documentation<StdlibItemId>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCandidate {
    pub item: &'static StdlibItem,
    pub type_arguments: Vec<(&'static str, BuiltinType)>,
}

impl CallCandidate {
    pub const fn receiver(&self) -> Option<TypeRef> {
        match self.item.kind {
            ItemKind::Method { receiver } => Some(receiver),
            ItemKind::Function | ItemKind::TypedFunction { .. } => None,
        }
    }
}

const VOID: TypeRef = TypeRef::Core(CoreTypeId::Void);
const U32: TypeRef = TypeRef::Core(CoreTypeId::U32);
const U64: TypeRef = TypeRef::Core(CoreTypeId::U64);
const I32: TypeRef = TypeRef::Core(CoreTypeId::I32);
const I64: TypeRef = TypeRef::Core(CoreTypeId::I64);
const F32: TypeRef = TypeRef::Core(CoreTypeId::F32);
const F64: TypeRef = TypeRef::Core(CoreTypeId::F64);
const ADDRESS: TypeRef = TypeRef::Core(CoreTypeId::Address);
const STRING: TypeRef = TypeRef::Standard(StdlibTypeId::String);
const SIGNATURE: TypeRef = TypeRef::Standard(StdlibTypeId::Signature);
const DURATION: TypeRef = TypeRef::Standard(StdlibTypeId::Duration);
const MODULE: TypeRef = TypeRef::Standard(StdlibTypeId::Module);
const UNITY_MODULE: TypeRef = TypeRef::Standard(StdlibTypeId::UnityModule);
const UNITY_IMAGE: TypeRef = TypeRef::Standard(StdlibTypeId::UnityImage);
const UNITY_CLASS: TypeRef = TypeRef::Standard(StdlibTypeId::UnityClass);
const UNITY_FIELD: TypeRef = TypeRef::Standard(StdlibTypeId::UnityField);
const TIMER_STATE: TypeRef = TypeRef::Standard(StdlibTypeId::TimerState);
const T_REF: TypeRef = TypeRef::Variable("T");
const T_RESULT: TypeRef = TypeRef::Result(&T_REF);
const ADDRESS_RESULT: TypeRef = TypeRef::Result(&ADDRESS);
const STRING_RESULT: TypeRef = TypeRef::Result(&STRING);
const STRING_ARRAY: TypeRef = TypeRef::Array(&STRING);
const U64_ARRAY: TypeRef = TypeRef::Array(&U64);
const T_ARRAY: TypeRef = TypeRef::Array(&T_REF);

const NUMERIC_PARAMETER: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[TypeConstraint::Numeric],
}];
const MEMORY_PARAMETER: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[TypeConstraint::MemoryReadable],
}];
const UNCONSTRAINED_T: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[],
}];

const PURE: &[Effect] = &[Effect::Pure];
const ALLOCATES: &[Effect] = &[Effect::Allocates];
const PROCESS: &[Effect] = &[Effect::ReadsProcess, Effect::RequiresAttachedProcess];
const PROCESS_SUSPEND: &[Effect] = &[
    Effect::ReadsProcess,
    Effect::RequiresAttachedProcess,
    Effect::Suspends,
    Effect::CancelsOnProcessClose,
];
const NEXT_TICK: &[Effect] = &[
    Effect::RequiresAttachedProcess,
    Effect::Suspends,
    Effect::CancelsOnProcessClose,
];
const TIMER_WRITE: &[Effect] = &[Effect::WritesTimer];
const TIMER_READ: &[Effect] = &[Effect::ReadsTimer];
const RUNTIME_WRITE: &[Effect] = &[Effect::WritesRuntime];
const MUTATES_VALUE: &[Effect] = &[Effect::MutatesValue];

const fn parameter(name: &'static str, ty: TypeRef, documentation: &'static str) -> Parameter {
    Parameter {
        name,
        ty,
        rule: ParameterRule::Value,
        documentation,
    }
}

const fn literal_parameter(
    name: &'static str,
    ty: TypeRef,
    rule: ParameterRule,
    documentation: &'static str,
) -> Parameter {
    Parameter {
        name,
        ty,
        rule,
        documentation,
    }
}

const BASIC_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let values = ["a", "b"]
    let joined = String.concat(values)
    print(joined)
    setVariable("Length", String.length(joined) as String)
    let state = timer.state()
    setTickRate(60.0)
}"#;
const DURATION_EXAMPLE: &str = r#"state "game.exe" {}
gameTime {
    return match timer.state() {
        TimerState.NotRunning => Duration.fromFrames(120, 60),
        TimerState.Running => Duration.fromParts(2, 0),
        _ => Duration.fromSeconds(2.0)
    }
}"#;
const NUMERIC_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let value: i32 = 9
    let minimum = value.min(7)
    let maximum = value.max(10)
    let bounded = value.clamp(0, 7)
}"#;
const ARRAY_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let bytes = [0x48u8, 0u8]
    bytes.set(1, 0x8bu8)
    let first = bytes.get(0)
    let count = bytes.length()
}"#;
const ADDRESS_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let base: address = 0x1000
    let field = base.offset(4)
    let next = field.add(8)
}"#;
const PROCESS_EXAMPLE: &str = r#"state "game.exe" {}
onAttach {
    let module = await process.module("GameAssembly.dll")
    let marker = await module.scan(sig"48 8B ?? 89")
    let rangedMarker = await process.scan(module.address, module.size, sig"48 8B ?? 89")
    let object = retry process.follow(module.address, [0x100, 0x20])
    let health = retry process.read.i32(object.offset(0x10))
    let target = retry process.readRelative32(marker.offset(3))
    let scene = retry process.read.managedString(target, 64)
    print(`{rangedMarker}:{health}:{scene}`)
}"#;
const UNITY_EXAMPLE: &str = r#"state "game.exe" {}
onAttach {
    let unity = await Unity.il2cpp(2020)
    let image = await unity.image("Assembly-CSharp")
    let gameManager = await image.class("GameManager")
    let healthOffset = await gameManager.field("health")
    let levelField = await gameManager.fieldAny(["currentLevel", "level"])
    let staticTable = await gameManager.staticTable()
    let instance = await gameManager.staticInstance(["Instance", "_instance"])
    print(`{healthOffset}:{levelField.offset}:{staticTable}:{instance}`)
}"#;
const NEXT_TICK_EXAMPLE: &str = r#"state "game.exe" {}
onAttach {
    await nextTick()
    print("Initialization resumed")
}"#;

macro_rules! doc_example {
    ($name:ident, $title:literal, $source:literal, $validation:expr) => {
        const $name: &[Example] = &[Example::checked($title, $source, $validation)];
    };
}

doc_example!(
    NUMERIC_MIN_EXAMPLE,
    "Keep the smaller value",
    "let visibleStage = stage.min(7)",
    NUMERIC_EXAMPLE
);
doc_example!(
    NUMERIC_MAX_EXAMPLE,
    "Keep the larger value",
    "let nonNegativeScore = score.max(0)",
    NUMERIC_EXAMPLE
);
doc_example!(
    NUMERIC_CLAMP_EXAMPLE,
    "Restrict a value to a range",
    "let visibleStage = stage.clamp(1, 7)",
    NUMERIC_EXAMPLE
);
doc_example!(
    PRINT_EXAMPLE,
    "Write to the runtime log",
    "print(\"Attached to the game\")",
    BASIC_EXAMPLE
);
doc_example!(
    SET_VARIABLE_EXAMPLE,
    "Expose a layout variable",
    "setVariable(\"Level\", levelName)",
    BASIC_EXAMPLE
);
doc_example!(
    SET_TICK_RATE_EXAMPLE,
    "Poll more frequently",
    "setTickRate(120.0)",
    BASIC_EXAMPLE
);
doc_example!(
    NEXT_TICK_DOC_EXAMPLE,
    "Resume on the next update",
    "await nextTick()",
    NEXT_TICK_EXAMPLE
);
doc_example!(
    STRING_LENGTH_EXAMPLE,
    "Measure UTF-8 text",
    "let byteLength = String.length(levelName)",
    BASIC_EXAMPLE
);
doc_example!(
    STRING_CONCAT_EXAMPLE,
    "Join strings",
    "let label = String.concat([\"Stage \", stageName])",
    BASIC_EXAMPLE
);
doc_example!(
    TIMER_STATE_EXAMPLE,
    "Check whether the timer is running",
    "let isRunning = timer.state() == TimerState.Running",
    BASIC_EXAMPLE
);
doc_example!(
    PROCESS_MODULE_EXAMPLE,
    "Wait for a module",
    "let gameAssembly = await process.module(\"GameAssembly.dll\")",
    PROCESS_EXAMPLE
);
doc_example!(
    PROCESS_READ_EXAMPLE,
    "Read a typed value",
    "let health = process.read.i32(player.offset(0x20)) else 0",
    PROCESS_EXAMPLE
);
doc_example!(
    PROCESS_FOLLOW_EXAMPLE,
    "Follow a pointer path",
    "let player = retry process.follow(module.address, [0x100, 0x20])",
    PROCESS_EXAMPLE
);
doc_example!(
    PROCESS_SCAN_EXAMPLE,
    "Scan a memory range",
    "let marker = await process.scan(module.address, module.size, sig\"48 8B ?? 89\")",
    PROCESS_EXAMPLE
);
doc_example!(
    PROCESS_RELATIVE_EXAMPLE,
    "Resolve a relative target",
    "let target = retry process.readRelative32(instruction.offset(3))",
    PROCESS_EXAMPLE
);
doc_example!(
    MANAGED_STRING_EXAMPLE,
    "Read a Unity string",
    "let scene = process.read.managedString(sceneAddress, 64) else \"Unknown\"",
    PROCESS_EXAMPLE
);
doc_example!(
    UNITY_IL2CPP_EXAMPLE,
    "Discover IL2CPP metadata",
    "let unity = await Unity.il2cpp(2020)",
    UNITY_EXAMPLE
);
doc_example!(
    DURATION_FRAMES_EXAMPLE,
    "Convert frames to game time",
    "return Duration.fromFrames(frameCount, 60)",
    DURATION_EXAMPLE
);
doc_example!(
    DURATION_PARTS_EXAMPLE,
    "Construct an exact duration",
    "return Duration.fromParts(seconds, nanoseconds)",
    DURATION_EXAMPLE
);
doc_example!(
    DURATION_SECONDS_EXAMPLE,
    "Convert seconds to game time",
    "return Duration.fromSeconds(elapsedSeconds)",
    DURATION_EXAMPLE
);
doc_example!(
    ADDRESS_OFFSET_EXAMPLE,
    "Add a field offset",
    "let healthAddress = player.offset(0x20)",
    ADDRESS_EXAMPLE
);
doc_example!(
    ADDRESS_ADD_EXAMPLE,
    "Add a full-width offset",
    "let target = module.address.add(sectionOffset)",
    ADDRESS_EXAMPLE
);
doc_example!(
    MODULE_SCAN_EXAMPLE,
    "Scan an entire module",
    "let marker = await gameAssembly.scan(sig\"48 8B ?? 89\")",
    PROCESS_EXAMPLE
);
doc_example!(
    UNITY_IMAGE_EXAMPLE,
    "Find a managed assembly",
    "let image = await unity.image(\"Assembly-CSharp\")",
    UNITY_EXAMPLE
);
doc_example!(
    UNITY_CLASS_EXAMPLE,
    "Find a managed class",
    "let gameManager = await image.class(\"GameManager\")",
    UNITY_EXAMPLE
);
doc_example!(
    UNITY_FIELD_EXAMPLE,
    "Find a field offset",
    "let healthOffset = await gameManager.field(\"health\")",
    UNITY_EXAMPLE
);
doc_example!(
    UNITY_FIELD_ANY_EXAMPLE,
    "Try multiple field names",
    "let levelField = await gameManager.fieldAny([\"currentLevel\", \"level\"])",
    UNITY_EXAMPLE
);
doc_example!(
    UNITY_STATIC_TABLE_EXAMPLE,
    "Find static storage",
    "let staticTable = await gameManager.staticTable()",
    UNITY_EXAMPLE
);
doc_example!(
    UNITY_STATIC_INSTANCE_EXAMPLE,
    "Find a singleton instance",
    "let instance = await gameManager.staticInstance([\"Instance\", \"_instance\"])",
    UNITY_EXAMPLE
);
doc_example!(
    ARRAY_LENGTH_EXAMPLE,
    "Count array elements",
    "let fieldCount = fieldNames.length()",
    ARRAY_EXAMPLE
);
doc_example!(
    ARRAY_GET_EXAMPLE,
    "Read an array element",
    "let firstField = fieldNames.get(0)",
    ARRAY_EXAMPLE
);
doc_example!(
    ARRAY_SET_EXAMPLE,
    "Replace an array element",
    "fieldNames.set(0, \"health\")",
    ARRAY_EXAMPLE
);

macro_rules! function_item {
    ($id:ident, $owner:expr, $name:literal, $qualified:literal, $params:expr, $result:expr,
     $effects:expr, $availability:expr, $summary:literal, $details:literal, $examples:expr) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: $qualified,
            kind: ItemKind::Function,
            signature: Signature {
                type_parameters: &[],
                parameters: $params,
                result: $result,
            },
            effects: $effects,
            availability: $availability,
            deprecation: None,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: $examples,
                related: &[],
            },
            implementation: Implementation::Intrinsic(IntrinsicId::$id),
        }
    };
    ($id:ident => $intrinsic:ident, $owner:expr, $name:literal, $qualified:literal, $params:expr,
     $result:expr, $effects:expr, $availability:expr, $summary:literal, $details:literal,
     $examples:expr) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: $qualified,
            kind: ItemKind::Function,
            signature: Signature {
                type_parameters: &[],
                parameters: $params,
                result: $result,
            },
            effects: $effects,
            availability: $availability,
            deprecation: None,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: $examples,
                related: &[],
            },
            implementation: Implementation::Intrinsic(IntrinsicId::$intrinsic),
        }
    };
}

macro_rules! method_item {
    ($id:ident, $owner:expr, $qualified:literal, $receiver:expr, $name:literal, $types:expr,
     $params:expr, $result:expr, $effects:expr, $availability:expr,
     $summary:literal, $details:literal, $examples:expr) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: $qualified,
            kind: ItemKind::Method {
                receiver: $receiver,
            },
            signature: Signature {
                type_parameters: $types,
                parameters: $params,
                result: $result,
            },
            effects: $effects,
            availability: $availability,
            deprecation: None,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: $examples,
                related: &[],
            },
            implementation: Implementation::Intrinsic(IntrinsicId::$id),
        }
    };
}

const ITEMS: &[StdlibItem] = &[
    method_item!(
        NumericMin,
        StdlibOwner::Capability(StdlibCapabilityId::Numeric),
        "Numeric.min",
        T_REF,
        "min",
        NUMERIC_PARAMETER,
        &[parameter(
            "other",
            T_REF,
            "The other value to compare with the receiver."
        )],
        T_REF,
        PURE,
        Availability::Everywhere,
        "Returns the smaller of two numeric values.",
        "Both values have the same inferred numeric type and are evaluated once.",
        NUMERIC_MIN_EXAMPLE
    ),
    method_item!(
        NumericMax,
        StdlibOwner::Capability(StdlibCapabilityId::Numeric),
        "Numeric.max",
        T_REF,
        "max",
        NUMERIC_PARAMETER,
        &[parameter(
            "other",
            T_REF,
            "The other value to compare with the receiver."
        )],
        T_REF,
        PURE,
        Availability::Everywhere,
        "Returns the larger of two numeric values.",
        "Both values have the same inferred numeric type and are evaluated once.",
        NUMERIC_MAX_EXAMPLE
    ),
    method_item!(
        NumericClamp,
        StdlibOwner::Capability(StdlibCapabilityId::Numeric),
        "Numeric.clamp",
        T_REF,
        "clamp",
        NUMERIC_PARAMETER,
        &[
            parameter("minimum", T_REF, "The inclusive lower bound."),
            parameter("maximum", T_REF, "The inclusive upper bound.")
        ],
        T_REF,
        PURE,
        Availability::Everywhere,
        "Restricts a numeric value to an inclusive range.",
        "The receiver and bounds have one inferred numeric type and are evaluated once.",
        NUMERIC_CLAMP_EXAMPLE
    ),
    function_item!(
        Print,
        StdlibOwner::Root,
        "print",
        "print",
        &[parameter(
            "message",
            STRING,
            "The message to write to the runtime log."
        )],
        VOID,
        RUNTIME_WRITE,
        Availability::Everywhere,
        "Prints a diagnostic message.",
        "The message is forwarded to the autosplitting runtime.",
        PRINT_EXAMPLE
    ),
    function_item!(
        SetVariable => TimerSetVariable,
        StdlibOwner::Root,
        "setVariable",
        "setVariable",
        &[
            parameter("name", STRING, "The variable name."),
            parameter("value", STRING, "The displayed value.")
        ],
        VOID,
        TIMER_WRITE,
        Availability::Everywhere,
        "Sets a LiveSplit custom variable.",
        "The value is visible to layouts that display autosplitter variables.",
        SET_VARIABLE_EXAMPLE
    ),
    function_item!(
        SetTickRate => RuntimeSetTickRate,
        StdlibOwner::Root,
        "setTickRate",
        "setTickRate",
        &[parameter("hz", F64, "The requested updates per second.")],
        VOID,
        RUNTIME_WRITE,
        Availability::Everywhere,
        "Changes the autosplitter tick rate.",
        "The runtime applies the requested polling frequency.",
        SET_TICK_RATE_EXAMPLE
    ),
    function_item!(
        NextTick,
        StdlibOwner::Root,
        "nextTick",
        "nextTick",
        &[],
        VOID,
        NEXT_TICK,
        Availability::OnAttach,
        "Continues attachment on the next runtime update.",
        "Always suspends once. The continuation resumes on the following attached-process tick and is cancelled if that process closes first.",
        NEXT_TICK_DOC_EXAMPLE
    ),
    function_item!(
        StringLength,
        StdlibOwner::Type(StdlibTypeId::String),
        "length",
        "String.length",
        &[parameter(
            "value",
            STRING,
            "The string whose UTF-8 byte length is returned."
        )],
        U32,
        PURE,
        Availability::Everywhere,
        "Returns a string's UTF-8 byte length.",
        "The result counts UTF-8 bytes in the current string representation.",
        STRING_LENGTH_EXAMPLE
    ),
    function_item!(
        StringConcat,
        StdlibOwner::Type(StdlibTypeId::String),
        "concat",
        "String.concat",
        &[parameter(
            "values",
            STRING_ARRAY,
            "The strings to concatenate in order."
        )],
        STRING,
        ALLOCATES,
        Availability::Everywhere,
        "Concatenates an array of strings.",
        "A new WebAssembly GC string is allocated.",
        STRING_CONCAT_EXAMPLE
    ),
    function_item!(
        TimerState,
        StdlibOwner::Namespace(StdlibNamespaceId::Timer),
        "state",
        "timer.state",
        &[],
        TIMER_STATE,
        TIMER_READ,
        Availability::Everywhere,
        "Returns the current timer state.",
        "Host states are converted to NotRunning, Running, Paused, Ended, or Unknown at the ABI boundary.",
        TIMER_STATE_EXAMPLE
    ),
    function_item!(
        ProcessModule,
        StdlibOwner::Namespace(StdlibNamespaceId::Process),
        "module",
        "process.module",
        &[literal_parameter(
            "name",
            STRING,
            ParameterRule::StringLiteral,
            "The exact module name."
        )],
        MODULE,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Waits for a process module.",
        "Suspends attachment until both module address and size are available.",
        PROCESS_MODULE_EXAMPLE
    ),
    StdlibItem {
        id: StdlibItemId::ProcessRead,
        owner: StdlibOwner::Namespace(StdlibNamespaceId::Process),
        name: "read",
        qualified_name: "process.read",
        kind: ItemKind::TypedFunction {
            type_parameter: "T",
        },
        signature: Signature {
            type_parameters: MEMORY_PARAMETER,
            parameters: &[parameter("address", ADDRESS, "The target address to read.")],
            result: T_RESULT,
        },
        effects: PROCESS,
        availability: Availability::Everywhere,
        deprecation: None,
        documentation: Documentation {
            summary: "Reads a fixed-layout value from process memory.",
            details: "The expected MemoryReadable type selects a fixed-size primitive or record layout. A synchronous read returns T!; retry polls until a value is available and yields T. Use a suffix such as process.read.i32 when context cannot determine a primitive type.",
            examples: PROCESS_READ_EXAMPLE,
            related: &[],
        },
        implementation: Implementation::Intrinsic(IntrinsicId::ProcessRead),
    },
    function_item!(
        ProcessFollow,
        StdlibOwner::Namespace(StdlibNamespaceId::Process),
        "follow",
        "process.follow",
        &[
            parameter("base", ADDRESS, "The initial address."),
            parameter("offsets", U64_ARRAY, "Pointer offsets to follow.")
        ],
        ADDRESS_RESULT,
        PROCESS,
        Availability::Everywhere,
        "Follows a pointer path.",
        "Each intermediate address is read as a 64-bit target pointer. A failed or null pointer read returns an error; use retry in onAttach to wait for success.",
        PROCESS_FOLLOW_EXAMPLE
    ),
    function_item!(
        ProcessScan,
        StdlibOwner::Namespace(StdlibNamespaceId::Process),
        "scan",
        "process.scan",
        &[
            parameter("address", ADDRESS, "The beginning of the range."),
            parameter("size", U64, "The number of bytes to scan."),
            literal_parameter(
                "signature",
                SIGNATURE,
                ParameterRule::SignatureLiteral,
                "The compile-time signature pattern."
            )
        ],
        ADDRESS,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Scans a process-memory range.",
        "Suspends until the signature is found in the requested range.",
        PROCESS_SCAN_EXAMPLE
    ),
    function_item!(
        ProcessReadRelative32,
        StdlibOwner::Namespace(StdlibNamespaceId::Process),
        "readRelative32",
        "process.readRelative32",
        &[parameter(
            "address",
            ADDRESS,
            "The address of a signed relative displacement."
        )],
        ADDRESS_RESULT,
        PROCESS,
        Availability::Everywhere,
        "Resolves a 32-bit relative address.",
        "Reads a signed displacement and adds it to the address following the displacement. A failed or null target returns an error; use retry in onAttach to wait for success.",
        PROCESS_RELATIVE_EXAMPLE
    ),
    function_item!(
        ProcessReadManagedString,
        StdlibOwner::Namespace(StdlibNamespaceId::ProcessRead),
        "managedString",
        "process.read.managedString",
        &[
            parameter("address", ADDRESS, "The managed string object address."),
            parameter(
                "maxUtf16Units",
                U32,
                "The maximum UTF-16 code units to decode."
            )
        ],
        STRING_RESULT,
        PROCESS,
        Availability::Everywhere,
        "Reads a Unity managed string.",
        "The bounded UTF-16 payload is decoded into an immutable SplitScript string. Memory-access failure returns an error; malformed surrogate sequences decode as the replacement character.",
        MANAGED_STRING_EXAMPLE
    ),
    function_item!(
        UnityIl2Cpp,
        StdlibOwner::Namespace(StdlibNamespaceId::Unity),
        "il2cpp",
        "Unity.il2cpp",
        &[parameter(
            "version",
            U32,
            "The Unity metadata layout version."
        )],
        UNITY_MODULE,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Discovers an IL2CPP runtime.",
        "Suspends until GameAssembly and the IL2CPP metadata structures are available.",
        UNITY_IL2CPP_EXAMPLE
    ),
    function_item!(
        DurationFromFrames,
        StdlibOwner::Type(StdlibTypeId::Duration),
        "fromFrames",
        "Duration.fromFrames",
        &[
            parameter("frames", I64, "The elapsed frame count."),
            parameter("framesPerSecond", I64, "The frame rate.")
        ],
        DURATION,
        PURE,
        Availability::Everywhere,
        "Constructs a duration from frames.",
        "The conversion preserves whole seconds and nanoseconds.",
        DURATION_FRAMES_EXAMPLE
    ),
    function_item!(
        DurationFromParts,
        StdlibOwner::Type(StdlibTypeId::Duration),
        "fromParts",
        "Duration.fromParts",
        &[
            parameter("seconds", I64, "Whole seconds."),
            parameter("nanoseconds", I32, "The fractional nanoseconds.")
        ],
        DURATION,
        PURE,
        Availability::Everywhere,
        "Constructs a duration from seconds and nanoseconds.",
        "The two components become the runtime duration representation.",
        DURATION_PARTS_EXAMPLE
    ),
    function_item!(
        DurationFromSeconds => DurationSaturatingSecondsF32,
        StdlibOwner::Type(StdlibTypeId::Duration),
        "fromSeconds",
        "Duration.fromSeconds",
        &[parameter("seconds", F32, "Floating-point seconds.")],
        DURATION,
        PURE,
        Availability::Everywhere,
        "Constructs a duration from floating-point seconds.",
        "Finite values are converted to the runtime duration representation; values outside its range are safely clamped.",
        DURATION_SECONDS_EXAMPLE
    ),
    method_item!(
        AddressOffset,
        StdlibOwner::Core(CoreTypeId::Address),
        "address.offset",
        ADDRESS,
        "offset",
        &[],
        &[parameter("offset", U32, "The unsigned field offset.")],
        ADDRESS,
        PURE,
        Availability::Everywhere,
        "Adds a field offset to an address.",
        "The offset is widened to the target address width.",
        ADDRESS_OFFSET_EXAMPLE
    ),
    method_item!(
        AddressAdd,
        StdlibOwner::Core(CoreTypeId::Address),
        "address.add",
        ADDRESS,
        "add",
        &[],
        &[parameter("offset", U64, "The full-width address offset.")],
        ADDRESS,
        PURE,
        Availability::Everywhere,
        "Adds a full-width offset to an address.",
        "This is useful for offsets that are already represented as u64.",
        ADDRESS_ADD_EXAMPLE
    ),
    method_item!(
        ModuleScan,
        StdlibOwner::Type(StdlibTypeId::Module),
        "Module.scan",
        MODULE,
        "scan",
        &[],
        &[literal_parameter(
            "signature",
            SIGNATURE,
            ParameterRule::SignatureLiteral,
            "The compile-time signature pattern."
        )],
        ADDRESS,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Scans a module for a signature.",
        "The module's address and size define the scanned range.",
        MODULE_SCAN_EXAMPLE
    ),
    method_item!(
        UnityModuleImage,
        StdlibOwner::Type(StdlibTypeId::UnityModule),
        "UnityModule.image",
        UNITY_MODULE,
        "image",
        &[],
        &[parameter("name", STRING, "The managed assembly name.")],
        UNITY_IMAGE,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds an IL2CPP image.",
        "Suspends until the named image is discoverable.",
        UNITY_IMAGE_EXAMPLE
    ),
    method_item!(
        UnityImageClass,
        StdlibOwner::Type(StdlibTypeId::UnityImage),
        "UnityImage.class",
        UNITY_IMAGE,
        "class",
        &[],
        &[parameter("name", STRING, "The managed class name.")],
        UNITY_CLASS,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds a class in an IL2CPP image.",
        "Suspends until the named class is discoverable.",
        UNITY_CLASS_EXAMPLE
    ),
    method_item!(
        UnityClassField,
        StdlibOwner::Type(StdlibTypeId::UnityClass),
        "UnityClass.field",
        UNITY_CLASS,
        "field",
        &[],
        &[parameter("name", STRING, "The managed field name.")],
        U32,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds a managed field offset.",
        "Searches the class hierarchy and recognizes backing fields.",
        UNITY_FIELD_EXAMPLE
    ),
    method_item!(
        UnityClassFieldAny,
        StdlibOwner::Type(StdlibTypeId::UnityClass),
        "UnityClass.fieldAny",
        UNITY_CLASS,
        "fieldAny",
        &[],
        &[parameter(
            "names",
            STRING_ARRAY,
            "Candidate field names in priority order."
        )],
        UNITY_FIELD,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds the first matching field.",
        "Returns both the field offset and selected candidate index.",
        UNITY_FIELD_ANY_EXAMPLE
    ),
    method_item!(
        UnityClassStaticTable,
        StdlibOwner::Type(StdlibTypeId::UnityClass),
        "UnityClass.staticTable",
        UNITY_CLASS,
        "staticTable",
        &[],
        &[],
        ADDRESS,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds a class's static-field table.",
        "Suspends until the static storage pointer is non-null.",
        UNITY_STATIC_TABLE_EXAMPLE
    ),
    method_item!(
        UnityClassStaticInstance,
        StdlibOwner::Type(StdlibTypeId::UnityClass),
        "UnityClass.staticInstance",
        UNITY_CLASS,
        "staticInstance",
        &[],
        &[parameter(
            "names",
            STRING_ARRAY,
            "Candidate singleton field names."
        )],
        ADDRESS,
        PROCESS_SUSPEND,
        Availability::OnAttach,
        "Finds a static singleton instance.",
        "Combines field discovery, static-table lookup, and a non-null pointer read.",
        UNITY_STATIC_INSTANCE_EXAMPLE
    ),
    method_item!(
        ArrayLength,
        StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Array),
        "Array.length",
        T_ARRAY,
        "length",
        UNCONSTRAINED_T,
        &[],
        U32,
        PURE,
        Availability::Everywhere,
        "Returns the number of array elements.",
        "The result is the WebAssembly GC array length.",
        ARRAY_LENGTH_EXAMPLE
    ),
    method_item!(
        ArrayGet,
        StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Array),
        "Array.get",
        T_ARRAY,
        "get",
        UNCONSTRAINED_T,
        &[parameter("index", U32, "The zero-based element index.")],
        T_REF,
        PURE,
        Availability::Everywhere,
        "Returns an array element.",
        "Indexing uses WebAssembly's bounds checks.",
        ARRAY_GET_EXAMPLE
    ),
    method_item!(
        ArraySet,
        StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Array),
        "Array.set",
        T_ARRAY,
        "set",
        UNCONSTRAINED_T,
        &[
            parameter("index", U32, "The zero-based element index."),
            parameter("value", T_REF, "The new element value.")
        ],
        VOID,
        MUTATES_VALUE,
        Availability::Everywhere,
        "Updates an array element.",
        "The array is evaluated once and updated in place.",
        ARRAY_SET_EXAMPLE
    ),
];

#[derive(Debug, Clone, Copy, Default)]
pub struct StandardLibrary;

impl StandardLibrary {
    pub const fn new() -> Self {
        Self
    }

    pub fn core_types(self) -> &'static [CoreType] {
        CORE_TYPES
    }

    pub fn core_type(self, id: CoreTypeId) -> &'static CoreType {
        CORE_TYPES
            .iter()
            .find(|ty| ty.id == id)
            .expect("every core type ID must have a declaration")
    }

    pub fn core_type_has_capability(self, ty: CoreTypeId, capability: StdlibCapabilityId) -> bool {
        self.core_type(ty).capabilities.contains(&capability)
    }

    pub fn namespaces(self) -> &'static [StdlibNamespace] {
        NAMESPACES
    }

    pub fn namespace(self, id: StdlibNamespaceId) -> &'static StdlibNamespace {
        NAMESPACES
            .iter()
            .find(|namespace| namespace.id == id)
            .expect("every standard-library namespace ID must have a declaration")
    }

    pub fn namespace_by_name(self, name: &str) -> Option<&'static StdlibNamespace> {
        NAMESPACES
            .iter()
            .find(|namespace| namespace.path.len() == 1 && namespace.name == name)
    }

    pub fn namespace_by_path(self, path: &[&str]) -> Option<&'static StdlibNamespace> {
        NAMESPACES.iter().find(|namespace| namespace.path == path)
    }

    pub fn types(self) -> &'static [StdlibType] {
        TYPES
    }

    pub fn type_decl(self, id: StdlibTypeId) -> &'static StdlibType {
        TYPES
            .iter()
            .find(|ty| ty.id == id)
            .expect("every standard-library type ID must have a declaration")
    }

    pub fn type_by_name(self, name: &str) -> Option<&'static StdlibType> {
        TYPES.iter().find(|ty| ty.name == name)
    }

    pub fn type_has_capability(self, ty: StdlibTypeId, capability: StdlibCapabilityId) -> bool {
        self.type_decl(ty).capabilities.contains(&capability)
    }

    pub fn fields(self) -> &'static [StdlibField] {
        FIELDS
    }

    pub fn field(self, id: StdlibFieldId) -> &'static StdlibField {
        FIELDS
            .iter()
            .find(|field| field.id == id)
            .expect("every standard-library field ID must have a declaration")
    }

    pub fn fields_of(self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibField> {
        FIELDS.iter().filter(move |field| field.owner == owner)
    }

    pub fn public_field(self, owner: StdlibTypeId, name: &str) -> Option<&'static StdlibField> {
        self.public_fields(owner).find(|field| field.name == name)
    }

    pub fn public_fields(self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibField> {
        self.fields_of(owner)
            .filter(|field| field.visibility == FieldVisibility::Public)
    }

    pub fn variants(self) -> &'static [StdlibVariant] {
        VARIANTS
    }

    pub fn variant(self, id: StdlibVariantId) -> &'static StdlibVariant {
        VARIANTS
            .iter()
            .find(|variant| variant.id == id)
            .expect("every standard-library variant ID must have a declaration")
    }

    pub fn variants_of(self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibVariant> {
        VARIANTS
            .iter()
            .filter(move |variant| variant.owner == owner)
    }

    pub fn items(self) -> &'static [StdlibItem] {
        ITEMS
    }

    pub fn item(self, id: StdlibItemId) -> &'static StdlibItem {
        ITEMS
            .iter()
            .find(|item| item.id == id)
            .expect("every standard-library ID must have a catalog entry")
    }

    pub fn item_by_name(self, qualified_name: &str) -> Option<&'static StdlibItem> {
        ITEMS
            .iter()
            .find(|item| item.qualified_name == qualified_name)
    }

    pub fn item_path(self, item: &StdlibItem) -> Option<Vec<&'static str>> {
        let mut path = match item.owner {
            StdlibOwner::Root => Vec::new(),
            StdlibOwner::Namespace(namespace) => self.namespace(namespace).path.to_vec(),
            StdlibOwner::Type(ty) => vec![self.type_decl(ty).name],
            StdlibOwner::Core(_) | StdlibOwner::Capability(_) | StdlibOwner::TypeConstructor(_) => {
                return None;
            }
        };
        path.push(item.name);
        Some(path)
    }

    pub fn function_candidates(self, path: &[String]) -> Vec<CallCandidate> {
        ITEMS
            .iter()
            .filter_map(|item| {
                let declared = self.item_path(item)?;
                match item.kind {
                    ItemKind::Function
                        if path.iter().map(String::as_str).eq(declared.iter().copied()) =>
                    {
                        Some(CallCandidate {
                            item,
                            type_arguments: Vec::new(),
                        })
                    }
                    ItemKind::TypedFunction { type_parameter }
                        if (path.len() == declared.len() || path.len() == declared.len() + 1)
                            && path[..declared.len()]
                                .iter()
                                .map(String::as_str)
                                .eq(declared.iter().copied()) =>
                    {
                        Some(CallCandidate {
                            item,
                            type_arguments: if path.len() == declared.len() {
                                Vec::new()
                            } else {
                                vec![(type_parameter, memory_type(&path[declared.len()])?)]
                            },
                        })
                    }
                    ItemKind::Function
                    | ItemKind::TypedFunction { .. }
                    | ItemKind::Method { .. } => None,
                }
            })
            .collect()
    }

    pub fn method_candidates(self, name: &str) -> Vec<CallCandidate> {
        ITEMS
            .iter()
            .filter_map(|item| {
                matches!(item.kind, ItemKind::Method { .. } if item.name == name).then_some(
                    CallCandidate {
                        item,
                        type_arguments: Vec::new(),
                    },
                )
            })
            .collect()
    }

    /// Returns method catalog entries applicable to a fully inferred receiver.
    pub fn methods_for_type(self, receiver: &TypeKind) -> Vec<&'static StdlibItem> {
        ITEMS
            .iter()
            .filter(|item| catalog_method_accepts(item, receiver))
            .collect()
    }

    pub fn resolve_path(self, path: &[String]) -> Option<CallCandidate> {
        self.function_candidates(path).into_iter().next()
    }

    pub fn render_signature(self, id: StdlibItemId) -> String {
        self.render_signature_with(id, &[])
    }

    /// Renders a catalog signature after replacing named type parameters with
    /// semantic types inferred at one call site.
    pub fn render_signature_with(
        self,
        id: StdlibItemId,
        substitutions: &[(&str, String)],
    ) -> String {
        let item = self.item(id);
        let signature = item.signature;
        let mut rendered = match item.kind {
            ItemKind::Function | ItemKind::TypedFunction { .. } => {
                format!("{}(", item.qualified_name)
            }
            ItemKind::Method { receiver } => {
                format!("{}.{}(", receiver.render_with(substitutions), item.name)
            }
        };
        for (index, parameter) in signature.parameters.iter().enumerate() {
            if index != 0 {
                rendered.push_str(", ");
            }
            rendered.push_str(parameter.name);
            rendered.push_str(": ");
            rendered.push_str(&parameter.ty.render_with(substitutions));
        }
        rendered.push_str(") -> ");
        rendered.push_str(&signature.result.render_with(substitutions));
        let unresolved = signature
            .type_parameters
            .iter()
            .filter(|parameter| {
                !substitutions
                    .iter()
                    .any(|(name, _)| *name == parameter.name)
            })
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            rendered.push_str(" where ");
            for (index, parameter) in unresolved.into_iter().enumerate() {
                if index != 0 {
                    rendered.push_str(", ");
                }
                rendered.push_str(parameter.name);
                if !parameter.constraints.is_empty() {
                    rendered.push_str(": ");
                }
                for (constraint_index, constraint) in parameter.constraints.iter().enumerate() {
                    if constraint_index != 0 {
                        rendered.push_str(" + ");
                    }
                    rendered.push_str(constraint.name());
                }
            }
        }
        rendered
    }

    pub fn render_operation_semantics(self, id: StdlibItemId) -> String {
        let semantics = self.item(id).operation_semantics();
        let mut facts = vec![match semantics.availability {
            Availability::Everywhere => "available everywhere",
            Availability::OnAttach => "available in onAttach",
        }];
        facts.push(match semantics.suspension {
            SuspensionKind::None => "synchronous",
            SuspensionKind::Retryable => "await retries until successful",
            SuspensionKind::Suspends => "suspends",
        });
        if semantics.requires_attached_process {
            facts.push("requires an attached process");
        }
        if semantics.cancellation == CancellationKind::ProcessClose {
            facts.push("cancels when the process closes");
        }
        facts.join("; ")
    }

    pub fn validate(self) -> Vec<String> {
        let mut errors = declarations::validate();
        let mut ids = HashSet::new();
        let mut qualified_names = HashSet::new();
        let mut call_shapes = HashSet::new();
        let mut example_sources = HashSet::new();
        for item in ITEMS {
            if !ids.insert(item.id) {
                errors.push(format!("duplicate standard-library ID `{:?}`", item.id));
            }
            if !qualified_names.insert(item.qualified_name) {
                errors.push(format!(
                    "duplicate standard-library name `{}`",
                    item.qualified_name
                ));
            }
            let path = self.item_path(item);
            let call_shape = match item.kind {
                ItemKind::Function => format!(
                    "function {}",
                    path.as_ref()
                        .expect("functions have source paths")
                        .join(".")
                ),
                ItemKind::TypedFunction { .. } => format!(
                    "typed function {}[.*]",
                    path.as_ref()
                        .expect("typed functions have source paths")
                        .join(".")
                ),
                ItemKind::Method { receiver } => {
                    format!("method {}.{}", receiver.render(), item.name)
                }
            };
            if let Some(path) = &path
                && path.join(".") != item.qualified_name
            {
                errors.push(format!(
                    "`{}` disagrees with its declared owner and name `{}`",
                    item.qualified_name,
                    path.join(".")
                ));
            }
            if !call_shapes.insert(call_shape.clone()) {
                errors.push(format!(
                    "duplicate standard-library call shape `{call_shape}`"
                ));
            }
            if item.documentation.summary.trim().is_empty() {
                errors.push(format!(
                    "`{}` has no documentation summary",
                    item.qualified_name
                ));
            }
            if item.documentation.details.trim().is_empty() {
                errors.push(format!(
                    "`{}` has no documentation details",
                    item.qualified_name
                ));
            }
            if item.documentation.examples.is_empty() {
                errors.push(format!("`{}` has no examples", item.qualified_name));
            }
            let example_call = match item.kind {
                ItemKind::Function => format!("{}(", item.qualified_name),
                ItemKind::TypedFunction { .. } => format!("{}.", item.qualified_name),
                ItemKind::Method { .. } => format!(".{}(", item.name),
            };
            for example in item.documentation.examples {
                if example.title.trim().is_empty()
                    || example.source.trim().is_empty()
                    || example.validation_source().trim().is_empty()
                {
                    errors.push(format!(
                        "`{}` has an incomplete example",
                        item.qualified_name
                    ));
                }
                if !example.source.contains(&example_call) {
                    errors.push(format!(
                        "example for `{}` does not demonstrate `{example_call}`",
                        item.qualified_name
                    ));
                }
                if !example_sources.insert(example.source) {
                    errors.push(format!(
                        "`{}` reuses another symbol's visible example",
                        item.qualified_name
                    ));
                }
            }
            let semantics = item.operation_semantics();
            if item.effects.contains(&Effect::Retryable) && item.effects.contains(&Effect::Suspends)
            {
                errors.push(format!(
                    "`{}` cannot be both retryable and intrinsically suspending",
                    item.qualified_name
                ));
            }
            if semantics.cancellation != CancellationKind::None
                && !semantics.suspension.is_awaitable()
            {
                errors.push(format!(
                    "`{}` is cancellable but not awaitable",
                    item.qualified_name
                ));
            }
            if semantics.cancellation == CancellationKind::ProcessClose
                && !semantics.requires_attached_process
            {
                errors.push(format!(
                    "`{}` cancels on process close but does not require a process",
                    item.qualified_name
                ));
            }
            if item.effects.contains(&Effect::ReadsProcess) && !semantics.requires_attached_process
            {
                errors.push(format!(
                    "`{}` reads process state but does not require an attached process",
                    item.qualified_name
                ));
            }
            if semantics.availability == Availability::OnAttach
                && !semantics.suspension.is_awaitable()
            {
                errors.push(format!(
                    "`{}` is onAttach-only but is not awaitable",
                    item.qualified_name
                ));
            }
            for parameter in item.signature.parameters {
                if parameter.documentation.trim().is_empty() {
                    errors.push(format!(
                        "parameter `{}.{}` has no documentation",
                        item.qualified_name, parameter.name
                    ));
                }
            }
            for related in item.documentation.related {
                if !ITEMS.iter().any(|candidate| candidate.id == *related) {
                    errors.push(format!(
                        "`{}` links to missing item `{:?}`",
                        item.qualified_name, related
                    ));
                }
            }
            if let Some(replacement) = item
                .deprecation
                .and_then(|deprecation| deprecation.replacement)
                && !ITEMS.iter().any(|candidate| candidate.id == replacement)
            {
                errors.push(format!(
                    "`{}` has missing replacement `{:?}`",
                    item.qualified_name, replacement
                ));
            }
        }
        errors
    }
}

fn catalog_method_accepts(item: &StdlibItem, receiver: &TypeKind) -> bool {
    let ItemKind::Method { receiver: declared } = item.kind else {
        return false;
    };
    match declared {
        TypeRef::Core(expected) => {
            matches!(receiver, TypeKind::Builtin(actual) if actual.core() == expected)
        }
        TypeRef::Array(_) => matches!(receiver, TypeKind::Array { .. }),
        TypeRef::Result(_) => matches!(receiver, TypeKind::Result { .. }),
        TypeRef::Standard(expected) => {
            matches!(receiver, TypeKind::Standard(actual) if *actual == expected)
        }
        TypeRef::Variable(name) => item
            .signature
            .type_parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .is_none_or(|parameter| {
                parameter
                    .constraints
                    .iter()
                    .all(|constraint| match constraint {
                        TypeConstraint::Numeric => {
                            semantic_type_has_capability(receiver, StdlibCapabilityId::Numeric)
                        }
                        TypeConstraint::MemoryReadable => semantic_type_has_capability(
                            receiver,
                            StdlibCapabilityId::MemoryReadable,
                        ),
                    })
            }),
    }
}

fn semantic_type_has_capability(ty: &TypeKind, capability: StdlibCapabilityId) -> bool {
    let library = StandardLibrary::new();
    match ty {
        TypeKind::Builtin(builtin) => library.core_type_has_capability(builtin.core(), capability),
        TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
        TypeKind::Record(_) => matches!(
            capability,
            StdlibCapabilityId::Equatable | StdlibCapabilityId::MemoryReadable
        ),
        TypeKind::Enum(_) | TypeKind::Option { .. } | TypeKind::Result { .. } => {
            capability == StdlibCapabilityId::Equatable
        }
        TypeKind::Array { .. } => false,
    }
}

fn memory_type(name: &str) -> Option<BuiltinType> {
    let library = StandardLibrary::new();
    library
        .core_types()
        .iter()
        .find(|ty| {
            ty.name == name
                && library.core_type_has_capability(ty.id, StdlibCapabilityId::MemoryReadable)
        })
        .map(|ty| BuiltinType::from_core(ty.id))
}
