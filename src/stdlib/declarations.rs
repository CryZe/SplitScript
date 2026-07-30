//! Nominal standard-library declarations shared by semantics, code generation,
//! documentation, and editor tooling.
//!
//! This module deliberately contains no parser, inference, or WebAssembly
//! encoder types. It describes public symbols and backend-neutral runtime
//! shapes; consumers resolve those identities into their own stage-specific
//! representations.

use std::{collections::HashSet, fmt};

use crate::catalog::Documentation;

use super::StdlibItemId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibNamespaceId {
    Process,
    ProcessRead,
    Timer,
    Unity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibTypeId {
    String,
    Signature,
    Duration,
    Module,
    TimerState,
    UnityModule,
    UnityImage,
    UnityClass,
    UnityField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibFieldId {
    DurationSeconds,
    DurationNanoseconds,
    ModuleAddress,
    ModuleSize,
    UnityModuleAssemblies,
    UnityModuleTypeInfoTable,
    UnityModuleVersion,
    UnityModulePointerSize,
    UnityImageAddress,
    UnityImageModule,
    UnityClassAddress,
    UnityClassModule,
    UnityFieldOffset,
    UnityFieldIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibVariantId {
    TimerStateNotRunning,
    TimerStateRunning,
    TimerStatePaused,
    TimerStateEnded,
    TimerStateUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibCapabilityId {
    Numeric,
    Equatable,
    Interpolatable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibTypeConstructorId {
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StdlibOwner {
    Root,
    Namespace(StdlibNamespaceId),
    Type(StdlibTypeId),
    Core(CoreTypeId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibSymbolId {
    Namespace(StdlibNamespaceId),
    Type(StdlibTypeId),
    Field(StdlibFieldId),
    Variant(StdlibVariantId),
    Item(StdlibItemId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreTypeId {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Address,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclaredTypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
}

impl fmt::Display for CoreTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Address => "address",
            Self::F32 => "f32",
            Self::F64 => "f64",
        })
    }
}

impl fmt::Display for DeclaredTypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(core) => core.fmt(formatter),
            Self::Standard(standard) => formatter.write_str(
                TYPES
                    .iter()
                    .find(|declaration| declaration.id == *standard)
                    .expect("every standard type reference has a declaration")
                    .name,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibTypeKind {
    Intrinsic,
    Struct,
    Enum,
}

/// Backend-neutral storage requirements for a standard-library type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRepresentation {
    Scalar {
        storage: CoreTypeId,
    },
    GcArray {
        element: CoreTypeId,
        mutable: bool,
        nullable: bool,
    },
    GcStruct {
        nullable: bool,
    },
    Enum {
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldVisibility {
    Public,
    RuntimePrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueUsage {
    pub record_field: bool,
    pub enum_payload: bool,
    pub state_field: bool,
    pub local_variable: bool,
    pub global_variable: bool,
}

const ORDINARY_LOCAL_VALUE: ValueUsage = ValueUsage {
    record_field: true,
    enum_payload: true,
    state_field: true,
    local_variable: true,
    global_variable: false,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibNamespace {
    pub id: StdlibNamespaceId,
    pub name: &'static str,
    pub path: &'static [&'static str],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibType {
    pub id: StdlibTypeId,
    pub name: &'static str,
    pub kind: StdlibTypeKind,
    pub capabilities: &'static [StdlibCapabilityId],
    pub representation: RuntimeRepresentation,
    pub value_usage: ValueUsage,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibField {
    pub id: StdlibFieldId,
    pub owner: StdlibTypeId,
    pub name: &'static str,
    pub ty: DeclaredTypeRef,
    pub visibility: FieldVisibility,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibVariant {
    pub id: StdlibVariantId,
    pub owner: StdlibTypeId,
    pub name: &'static str,
    pub documentation: Documentation<StdlibSymbolId>,
}

const fn documentation(
    summary: &'static str,
    details: &'static str,
) -> Documentation<StdlibSymbolId> {
    Documentation {
        summary,
        details,
        examples: &[],
        related: &[],
    }
}

pub(super) const NAMESPACES: &[StdlibNamespace] = &[
    StdlibNamespace {
        id: StdlibNamespaceId::Process,
        name: "process",
        path: &["process"],
        documentation: documentation(
            "Accesses the attached game process.",
            "Process operations discover modules, read memory, follow pointers, and scan signatures.",
        ),
    },
    StdlibNamespace {
        id: StdlibNamespaceId::ProcessRead,
        name: "read",
        path: &["process", "read"],
        documentation: documentation(
            "Reads typed values from process memory.",
            "The expected type or an explicit suffix selects the process-memory layout.",
        ),
    },
    StdlibNamespace {
        id: StdlibNamespaceId::Timer,
        name: "timer",
        path: &["timer"],
        documentation: documentation(
            "Reads information from the LiveSplit timer.",
            "Timer operations expose runtime state used by autosplitter decisions.",
        ),
    },
    StdlibNamespace {
        id: StdlibNamespaceId::Unity,
        name: "Unity",
        path: &["Unity"],
        documentation: documentation(
            "Discovers and inspects Unity runtimes.",
            "Unity operations attach to IL2CPP metadata and produce typed images, classes, and fields.",
        ),
    },
];

const EQUATABLE_INTERPOLATABLE: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::Interpolatable,
];
const EQUATABLE: &[StdlibCapabilityId] = &[StdlibCapabilityId::Equatable];

pub(super) const TYPES: &[StdlibType] = &[
    StdlibType {
        id: StdlibTypeId::String,
        name: "String",
        kind: StdlibTypeKind::Intrinsic,
        capabilities: EQUATABLE_INTERPOLATABLE,
        representation: RuntimeRepresentation::GcArray {
            element: CoreTypeId::U8,
            mutable: true,
            nullable: true,
        },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Stores immutable UTF-8 text.",
            "String literals, interpolation, process decoders, and timer variables use garbage-collected strings.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::Signature,
        name: "Signature",
        kind: StdlibTypeKind::Intrinsic,
        capabilities: &[],
        representation: RuntimeRepresentation::Scalar {
            storage: CoreTypeId::I64,
        },
        value_usage: ValueUsage {
            record_field: false,
            enum_payload: false,
            state_field: false,
            local_variable: false,
            global_variable: false,
        },
        documentation: documentation(
            "Stores a compiled process-memory signature.",
            "Signature values are created by signature literals and consumed by scanning operations.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::Duration,
        name: "Duration",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: false },
        value_usage: ValueUsage {
            record_field: true,
            enum_payload: false,
            state_field: false,
            local_variable: false,
            global_variable: false,
        },
        documentation: documentation(
            "Represents a precise span of time.",
            "Durations carry whole seconds and nanoseconds and are used for LiveSplit game time.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::Module,
        name: "Module",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: true },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Describes a module loaded in the attached process.",
            "A module exposes its base address and mapped size for bounded memory discovery.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::TimerState,
        name: "TimerState",
        kind: StdlibTypeKind::Enum,
        capabilities: EQUATABLE,
        representation: RuntimeRepresentation::Enum { nullable: true },
        value_usage: ValueUsage {
            global_variable: true,
            ..ORDINARY_LOCAL_VALUE
        },
        documentation: documentation(
            "Describes the current LiveSplit timer state.",
            "Timer state is an exhaustive enum returned by timer.state().",
        ),
    },
    StdlibType {
        id: StdlibTypeId::UnityModule,
        name: "UnityModule",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: true },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Describes an attached Unity IL2CPP runtime.",
            "The runtime stores resolved metadata roots, its version, and pointer size.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::UnityImage,
        name: "UnityImage",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: true },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Describes a Unity assembly image.",
            "An image retains its owning Unity runtime for subsequent class lookup.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::UnityClass,
        name: "UnityClass",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: true },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Describes a Unity runtime class.",
            "A class retains its owning Unity runtime for field and static-data discovery.",
        ),
    },
    StdlibType {
        id: StdlibTypeId::UnityField,
        name: "UnityField",
        kind: StdlibTypeKind::Struct,
        capabilities: &[],
        representation: RuntimeRepresentation::GcStruct { nullable: true },
        value_usage: ORDINARY_LOCAL_VALUE,
        documentation: documentation(
            "Describes a Unity runtime field.",
            "A field exposes its byte offset and metadata index.",
        ),
    },
];

macro_rules! field {
    ($id:ident, $owner:ident, $name:literal, $ty:expr, $visibility:ident, $summary:literal) => {
        StdlibField {
            id: StdlibFieldId::$id,
            owner: StdlibTypeId::$owner,
            name: $name,
            ty: $ty,
            visibility: FieldVisibility::$visibility,
            documentation: documentation($summary, $summary),
        }
    };
}

pub(super) const FIELDS: &[StdlibField] = &[
    field!(
        DurationSeconds,
        Duration,
        "seconds",
        DeclaredTypeRef::Core(CoreTypeId::I64),
        RuntimePrivate,
        "Stores the whole-second component."
    ),
    field!(
        DurationNanoseconds,
        Duration,
        "nanoseconds",
        DeclaredTypeRef::Core(CoreTypeId::I32),
        RuntimePrivate,
        "Stores the fractional nanosecond component."
    ),
    field!(
        ModuleAddress,
        Module,
        "address",
        DeclaredTypeRef::Core(CoreTypeId::Address),
        Public,
        "Returns the module base address."
    ),
    field!(
        ModuleSize,
        Module,
        "size",
        DeclaredTypeRef::Core(CoreTypeId::U64),
        Public,
        "Returns the mapped module size."
    ),
    field!(
        UnityModuleAssemblies,
        UnityModule,
        "assemblies",
        DeclaredTypeRef::Core(CoreTypeId::Address),
        Public,
        "Returns the IL2CPP assemblies metadata address."
    ),
    field!(
        UnityModuleTypeInfoTable,
        UnityModule,
        "typeInfoTable",
        DeclaredTypeRef::Core(CoreTypeId::Address),
        Public,
        "Returns the IL2CPP type-information table address."
    ),
    field!(
        UnityModuleVersion,
        UnityModule,
        "version",
        DeclaredTypeRef::Core(CoreTypeId::U32),
        Public,
        "Returns the detected Unity metadata version."
    ),
    field!(
        UnityModulePointerSize,
        UnityModule,
        "pointerSize",
        DeclaredTypeRef::Core(CoreTypeId::U32),
        Public,
        "Returns the attached process pointer size."
    ),
    field!(
        UnityImageAddress,
        UnityImage,
        "address",
        DeclaredTypeRef::Core(CoreTypeId::Address),
        Public,
        "Returns the Unity image address."
    ),
    field!(
        UnityImageModule,
        UnityImage,
        "module",
        DeclaredTypeRef::Standard(StdlibTypeId::UnityModule),
        RuntimePrivate,
        "Retains the owning Unity runtime."
    ),
    field!(
        UnityClassAddress,
        UnityClass,
        "address",
        DeclaredTypeRef::Core(CoreTypeId::Address),
        Public,
        "Returns the Unity class address."
    ),
    field!(
        UnityClassModule,
        UnityClass,
        "module",
        DeclaredTypeRef::Standard(StdlibTypeId::UnityModule),
        RuntimePrivate,
        "Retains the owning Unity runtime."
    ),
    field!(
        UnityFieldOffset,
        UnityField,
        "offset",
        DeclaredTypeRef::Core(CoreTypeId::U32),
        Public,
        "Returns the instance-field byte offset."
    ),
    field!(
        UnityFieldIndex,
        UnityField,
        "index",
        DeclaredTypeRef::Core(CoreTypeId::U32),
        Public,
        "Returns the metadata field index."
    ),
];

macro_rules! variant {
    ($id:ident, $name:literal, $summary:literal) => {
        StdlibVariant {
            id: StdlibVariantId::$id,
            owner: StdlibTypeId::TimerState,
            name: $name,
            documentation: documentation($summary, $summary),
        }
    };
}

pub(super) const VARIANTS: &[StdlibVariant] = &[
    variant!(
        TimerStateNotRunning,
        "NotRunning",
        "The timer has not started."
    ),
    variant!(TimerStateRunning, "Running", "The timer is running."),
    variant!(TimerStatePaused, "Paused", "The timer is paused."),
    variant!(TimerStateEnded, "Ended", "The timer has ended."),
    variant!(
        TimerStateUnknown,
        "Unknown",
        "The host returned an unknown timer state."
    ),
];

pub(super) fn validate() -> Vec<String> {
    let mut errors = Vec::new();
    let mut namespace_ids = HashSet::new();
    let mut namespace_names = HashSet::new();
    let mut namespace_paths = HashSet::new();
    for namespace in NAMESPACES {
        if !namespace_ids.insert(namespace.id) {
            errors.push(format!("duplicate namespace ID `{:?}`", namespace.id));
        }
        let parent = &namespace.path[..namespace.path.len().saturating_sub(1)];
        if !namespace_names.insert((parent, namespace.name)) {
            errors.push(format!(
                "duplicate namespace member `{}` below `{}`",
                namespace.name,
                parent.join(".")
            ));
        }
        if !namespace_paths.insert(namespace.path) {
            errors.push(format!(
                "duplicate namespace path `{}`",
                namespace.path.join(".")
            ));
        }
        if namespace.path.last().copied() != Some(namespace.name) {
            errors.push(format!(
                "namespace `{:?}` has path/name disagreement",
                namespace.id
            ));
        }
        if namespace.path.len() > 1 && !NAMESPACES.iter().any(|candidate| candidate.path == parent)
        {
            errors.push(format!(
                "namespace `{}` has missing parent `{}`",
                namespace.path.join("."),
                parent.join(".")
            ));
        }
        validate_documentation(
            &mut errors,
            "namespace",
            namespace.name,
            &namespace.documentation,
        );
    }

    let mut type_ids = HashSet::new();
    let mut type_names = HashSet::new();
    for ty in TYPES {
        if !type_ids.insert(ty.id) {
            errors.push(format!("duplicate standard type ID `{:?}`", ty.id));
        }
        if !type_names.insert(ty.name) {
            errors.push(format!("duplicate standard type name `{}`", ty.name));
        }
        validate_documentation(&mut errors, "type", ty.name, &ty.documentation);
        let has_fields = FIELDS.iter().any(|field| field.owner == ty.id);
        let has_variants = VARIANTS.iter().any(|variant| variant.owner == ty.id);
        match ty.kind {
            StdlibTypeKind::Enum if !has_variants => {
                errors.push(format!("enum `{}` has no variants", ty.name));
            }
            StdlibTypeKind::Enum if has_fields => {
                errors.push(format!("enum `{}` declares struct fields", ty.name));
            }
            StdlibTypeKind::Struct if !has_fields => {
                errors.push(format!("struct `{}` has no fields", ty.name));
            }
            StdlibTypeKind::Intrinsic | StdlibTypeKind::Struct | StdlibTypeKind::Enum => {}
        }
    }

    let mut field_ids = HashSet::new();
    let mut field_names = HashSet::new();
    for field in FIELDS {
        if !field_ids.insert(field.id) {
            errors.push(format!("duplicate standard field ID `{:?}`", field.id));
        }
        if !field_names.insert((field.owner, field.name)) {
            errors.push(format!(
                "duplicate field name `{:?}.{}`",
                field.owner, field.name
            ));
        }
        validate_documentation(&mut errors, "field", field.name, &field.documentation);
        if !type_ids.contains(&field.owner) {
            errors.push(format!(
                "field `{:?}` has missing owner `{:?}`",
                field.id, field.owner
            ));
        }
        if let DeclaredTypeRef::Standard(referenced) = field.ty
            && !type_ids.contains(&referenced)
        {
            errors.push(format!(
                "field `{:?}` references missing type `{:?}`",
                field.id, referenced
            ));
        }
    }

    let mut variant_ids = HashSet::new();
    let mut variant_names = HashSet::new();
    for variant in VARIANTS {
        if !variant_ids.insert(variant.id) {
            errors.push(format!("duplicate standard variant ID `{:?}`", variant.id));
        }
        if !variant_names.insert((variant.owner, variant.name)) {
            errors.push(format!(
                "duplicate variant name `{:?}.{}`",
                variant.owner, variant.name
            ));
        }
        validate_documentation(&mut errors, "variant", variant.name, &variant.documentation);
        if !type_ids.contains(&variant.owner) {
            errors.push(format!(
                "variant `{:?}` has missing owner `{:?}`",
                variant.id, variant.owner
            ));
        }
    }

    errors
}

fn validate_documentation<Id>(
    errors: &mut Vec<String>,
    kind: &str,
    name: &str,
    documentation: &Documentation<Id>,
) {
    if documentation.summary.trim().is_empty() {
        errors.push(format!("{kind} `{name}` has no documentation summary"));
    }
    if documentation.details.trim().is_empty() {
        errors.push(format!("{kind} `{name}` has no documentation details"));
    }
}
