//! Authored standard-library hierarchy and its generated flat symbol tables.
//!
//! This Rust declaration layer is intentionally an adapter: compiler and
//! tooling consumers depend on the normalized graph it emits, so a future
//! privileged SplitScript standard-library loader can replace this producer
//! without changing those consumers.

use super::declarations::{EQUATABLE, EQUATABLE_INTERPOLATABLE, ORDINARY_LOCAL_VALUE};
use super::*;

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

macro_rules! function_item {
    (@root $id:ident, $intrinsic:ident, $name:literal, $params:expr, $result:expr,
     $effects:expr, $availability:expr, $summary:literal, $details:literal, $examples:expr) => {
        function_item!(@emit $id, $intrinsic, StdlibOwner::Root, $name, $name,
            $params, $result, $effects, $availability, $summary, $details, $examples)
    };
    (@owned $id:ident, $intrinsic:ident, $owner:expr, $prefix:literal, $receiver:expr,
     $name:literal, $params:expr, $result:expr, $effects:expr, $availability:expr,
     $summary:literal, $details:literal, $examples:expr) => {
        function_item!(@emit $id, $intrinsic, $owner, $name, concat!($prefix, ".", $name),
            $params, $result, $effects, $availability, $summary, $details, $examples)
    };
    (@emit $id:ident, $intrinsic:ident, $owner:expr, $name:literal, $qualified:expr,
     $params:expr, $result:expr, $effects:expr, $availability:expr,
     $summary:literal, $details:literal, $examples:expr) => {
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

macro_rules! typed_function_item {
    (@owned $id:ident, $intrinsic:ident, $owner:expr, $prefix:literal, $receiver:expr,
     $name:literal, $type_parameter:literal, $types:expr, $params:expr, $result:expr,
     $effects:expr, $availability:expr, $summary:literal, $details:literal, $examples:expr) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: concat!($prefix, ".", $name),
            kind: ItemKind::TypedFunction {
                type_parameter: $type_parameter,
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
            implementation: Implementation::Intrinsic(IntrinsicId::$intrinsic),
        }
    };
}

macro_rules! method_item {
    (@owned $id:ident, $intrinsic:ident, $owner:expr, $prefix:literal, $receiver:expr,
     $name:literal, $types:expr, $params:expr, $result:expr, $effects:expr,
     $availability:expr, $summary:literal, $details:literal, $examples:expr) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: concat!($prefix, ".", $name),
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
            implementation: Implementation::Intrinsic(IntrinsicId::$intrinsic),
        }
    };
}

macro_rules! standard_library {
    (
        root {
            items { $($root_item_id:ident => intrinsic $root_intrinsic:ident,
                $root_factory:ident!($($root_arguments:tt)*)),* $(,)? }
        }
        capabilities {
            $($capability_id:ident => {
                name: $capability_name:literal,
                receiver: $capability_receiver:expr,
                summary: $capability_summary:literal,
                details: $capability_details:literal,
                items { $($capability_item_id:ident => intrinsic $capability_intrinsic:ident,
                    $capability_factory:ident!($($capability_arguments:tt)*)),* $(,)? }
            }),* $(,)?
        }
        type_constructors {
            $($constructor_id:ident => {
                name: $constructor_name:literal,
                receiver: $constructor_receiver:expr,
                summary: $constructor_summary:literal,
                details: $constructor_details:literal,
                items { $($constructor_item_id:ident => intrinsic $constructor_intrinsic:ident,
                    $constructor_factory:ident!($($constructor_arguments:tt)*)),* $(,)? }
            }),* $(,)?
        }
        core_extensions {
            $($core_id:ident => {
                name: $core_name:literal,
                items { $($core_item_id:ident => intrinsic $core_intrinsic:ident,
                    $core_factory:ident!($($core_arguments:tt)*)),* $(,)? }
            }),* $(,)?
        }
        namespaces {
            $($namespace_id:ident => {
                name: $namespace_name:literal,
                path: $namespace_path:expr,
                qualified: $namespace_qualified:literal,
                summary: $namespace_summary:literal,
                details: $namespace_details:literal,
                items { $($namespace_item_id:ident => intrinsic $namespace_intrinsic:ident,
                    $namespace_factory:ident!($($namespace_arguments:tt)*)),* $(,)? }
            }),* $(,)?
        }
        types {
            $(
                $(#[$type_attribute:meta])*
                $type_id:ident => {
                    name: $type_name:literal,
                    kind: $type_kind:ident,
                    capabilities: $type_capabilities:expr,
                    representation: $type_representation:expr,
                    value_usage: $type_value_usage:expr,
                    summary: $type_summary:literal,
                    details: $type_details:literal,
                    fields {
                        $($(#[$field_attribute:meta])*
                            $field_id:ident => {
                                name: $field_name:literal,
                                ty: $field_type:expr,
                                visibility: $field_visibility:ident,
                                docs: $field_docs:literal $(,)?
                            }
                        ),* $(,)?
                    }
                    variants {
                        $($(#[$variant_attribute:meta])*
                            $variant_id:ident => {
                                name: $variant_name:literal,
                                docs: $variant_docs:literal $(,)?
                            }
                        ),* $(,)?
                    }
                    items {
                        $($type_item_id:ident => intrinsic $type_intrinsic:ident,
                            $type_factory:ident!($($type_arguments:tt)*)),* $(,)?
                    }
                }
            ),* $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibCapabilityId { $($capability_id),* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibTypeConstructorId { $($constructor_id),* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibNamespaceId { $($namespace_id),* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibTypeId { $($(#[$type_attribute])* $type_id),* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibFieldId { $($($(#[$field_attribute])* $field_id,)*)* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibVariantId { $($($(#[$variant_attribute])* $variant_id,)*)* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum StdlibItemId {
            $($root_item_id,)*
            $($($capability_item_id,)*)*
            $($($constructor_item_id,)*)*
            $($($core_item_id,)*)*
            $($($namespace_item_id,)*)*
            $($($type_item_id,)*)*
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IntrinsicId {
            $($root_intrinsic,)*
            $($($capability_intrinsic,)*)*
            $($($constructor_intrinsic,)*)*
            $($($core_intrinsic,)*)*
            $($($namespace_intrinsic,)*)*
            $($($type_intrinsic,)*)*
        }

        pub(super) const NAMESPACES: &[StdlibNamespace] = &[
            $(StdlibNamespace {
                id: StdlibNamespaceId::$namespace_id,
                name: $namespace_name,
                path: $namespace_path,
                documentation: documentation($namespace_summary, $namespace_details),
            }),*
        ];

        pub(super) const CAPABILITIES: &[StdlibCapability] = &[
            $(StdlibCapability {
                id: StdlibCapabilityId::$capability_id,
                name: $capability_name,
                documentation: documentation($capability_summary, $capability_details),
            }),*
        ];

        pub(super) const TYPE_CONSTRUCTORS: &[StdlibTypeConstructor] = &[
            $(StdlibTypeConstructor {
                id: StdlibTypeConstructorId::$constructor_id,
                name: $constructor_name,
                documentation: documentation($constructor_summary, $constructor_details),
            }),*
        ];

        pub(super) const TYPES: &[StdlibType] = &[
            $($(#[$type_attribute])* StdlibType {
                id: StdlibTypeId::$type_id,
                name: $type_name,
                kind: StdlibTypeKind::$type_kind,
                capabilities: $type_capabilities,
                representation: $type_representation,
                value_usage: $type_value_usage,
                documentation: documentation($type_summary, $type_details),
            }),*
        ];

        pub(super) const FIELDS: &[StdlibField] = &[
            $($($(#[$field_attribute])* StdlibField {
                id: StdlibFieldId::$field_id,
                owner: StdlibTypeId::$type_id,
                name: $field_name,
                ty: $field_type,
                visibility: FieldVisibility::$field_visibility,
                documentation: documentation($field_docs, $field_docs),
            },)*)*
        ];

        pub(super) const VARIANTS: &[StdlibVariant] = &[
            $($($(#[$variant_attribute])* StdlibVariant {
                id: StdlibVariantId::$variant_id,
                owner: StdlibTypeId::$type_id,
                name: $variant_name,
                documentation: documentation($variant_docs, $variant_docs),
            },)*)*
        ];

        pub(super) const ITEMS: &[StdlibItem] = &[
            $($root_factory!(@root $root_item_id, $root_intrinsic, $($root_arguments)*),)*
            $($($capability_factory!(@owned $capability_item_id, $capability_intrinsic,
                StdlibOwner::Capability(StdlibCapabilityId::$capability_id),
                $capability_name, $capability_receiver, $($capability_arguments)*),)*)*
            $($($constructor_factory!(@owned $constructor_item_id, $constructor_intrinsic,
                StdlibOwner::TypeConstructor(StdlibTypeConstructorId::$constructor_id),
                $constructor_name, $constructor_receiver, $($constructor_arguments)*),)*)*
            $($($core_factory!(@owned $core_item_id, $core_intrinsic,
                StdlibOwner::Core(CoreTypeId::$core_id), $core_name,
                TypeRef::Core(CoreTypeId::$core_id), $($core_arguments)*),)*)*
            $($($namespace_factory!(@owned $namespace_item_id, $namespace_intrinsic,
                StdlibOwner::Namespace(StdlibNamespaceId::$namespace_id),
                $namespace_qualified, VOID, $($namespace_arguments)*),)*)*
            $($($type_factory!(@owned $type_item_id, $type_intrinsic,
                StdlibOwner::Type(StdlibTypeId::$type_id), $type_name,
                TypeRef::Standard(StdlibTypeId::$type_id), $($type_arguments)*),)*)*
        ];
    };
}

standard_library! {
    root {
        items {
            Print => intrinsic Print, function_item!(
                "print",
                &[parameter("message", STRING, "The message to write to the runtime log.")],
                VOID,
                RUNTIME_WRITE,
                Availability::Everywhere,
                "Prints a diagnostic message.",
                "The message is forwarded to the autosplitting runtime.",
                PRINT_EXAMPLE
            ),
            SetVariable => intrinsic TimerSetVariable, function_item!(
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
            SetTickRate => intrinsic RuntimeSetTickRate, function_item!(
                "setTickRate",
                &[parameter("hz", F64, "The requested updates per second.")],
                VOID,
                RUNTIME_WRITE,
                Availability::Everywhere,
                "Changes the autosplitter tick rate.",
                "The runtime applies the requested polling frequency.",
                SET_TICK_RATE_EXAMPLE
            ),
            NextTick => intrinsic NextTick, function_item!(
                "nextTick",
                &[],
                VOID,
                NEXT_TICK,
                Availability::OnAttach,
                "Continues attachment on the next runtime update.",
                "Always suspends once. The continuation resumes on the following attached-process tick and is cancelled if that process closes first.",
                NEXT_TICK_DOC_EXAMPLE
            )
        }
    }
    capabilities {
        Numeric => {
            name: "Numeric",
            receiver: T_REF,
            summary: "Supports ordered numeric operations.",
            details: "Numeric values provide minimum, maximum, and clamping operations while preserving their inferred type.",
            items {
                NumericMin => intrinsic NumericMin, method_item!(
                    "min",
                    NUMERIC_PARAMETER,
                    &[parameter("other", T_REF, "The other value to compare with the receiver.")],
                    T_REF,
                    PURE,
                    Availability::Everywhere,
                    "Returns the smaller of two numeric values.",
                    "Both values have the same inferred numeric type and are evaluated once.",
                    NUMERIC_MIN_EXAMPLE
                ),
                NumericMax => intrinsic NumericMax, method_item!(
                    "max",
                    NUMERIC_PARAMETER,
                    &[parameter("other", T_REF, "The other value to compare with the receiver.")],
                    T_REF,
                    PURE,
                    Availability::Everywhere,
                    "Returns the larger of two numeric values.",
                    "Both values have the same inferred numeric type and are evaluated once.",
                    NUMERIC_MAX_EXAMPLE
                ),
                NumericClamp => intrinsic NumericClamp, method_item!(
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
                )
            }
        },
        Integer => {
            name: "Integer", receiver: T_REF,
            summary: "Identifies integral numeric types.",
            details: "Integer operations and inference constraints accept only whole-number representations.",
            items {}
        },
        Signed => {
            name: "Signed", receiver: T_REF,
            summary: "Identifies signed numeric types.",
            details: "Signed values can represent numbers below zero.",
            items {}
        },
        Float => {
            name: "Float", receiver: T_REF,
            summary: "Identifies floating-point types.",
            details: "Floating-point constraints accept f32 and f64 values.",
            items {}
        },
        Equatable => {
            name: "Equatable", receiver: T_REF,
            summary: "Supports equality comparison.",
            details: "Equality is available for values whose representation can be compared safely.",
            items {}
        },
        StringCast => {
            name: "StringCast", receiver: T_REF,
            summary: "Supports explicit conversion to String.",
            details: "Values with this capability may be converted with an `as String` cast.",
            items {}
        },
        Interpolatable => {
            name: "Interpolatable", receiver: T_REF,
            summary: "Supports insertion into interpolated strings.",
            details: "Interpolated strings convert values with this capability into their textual representation.",
            items {}
        },
        MemoryReadable => {
            name: "MemoryReadable", receiver: T_REF,
            summary: "Supports deserialization from process memory.",
            details: "Primitive and record layouts with this capability can be selected by `process.read`.",
            items {}
        }
    }
    type_constructors {
        Array => {
            name: "Array",
            receiver: T_ARRAY,
            summary: "Stores an indexed sequence of values.",
            details: "Arrays use WebAssembly GC storage and expose generic length, access, and mutation operations.",
            items {
                ArrayLength => intrinsic ArrayLength, method_item!(
                    "length", UNCONSTRAINED_T, &[], U32, PURE, Availability::Everywhere,
                    "Returns the number of array elements.",
                    "The result is the WebAssembly GC array length.",
                    ARRAY_LENGTH_EXAMPLE
                ),
                ArrayGet => intrinsic ArrayGet, method_item!(
                    "get", UNCONSTRAINED_T,
                    &[parameter("index", U32, "The zero-based element index.")],
                    T_REF, PURE, Availability::Everywhere,
                    "Returns an array element.",
                    "Indexing uses WebAssembly's bounds checks.",
                    ARRAY_GET_EXAMPLE
                ),
                ArraySet => intrinsic ArraySet, method_item!(
                    "set", UNCONSTRAINED_T,
                    &[
                        parameter("index", U32, "The zero-based element index."),
                        parameter("value", T_REF, "The new element value.")
                    ],
                    VOID, MUTATES_VALUE, Availability::Everywhere,
                    "Updates an array element.",
                    "The array is evaluated once and updated in place.",
                    ARRAY_SET_EXAMPLE
                )
            }
        }
    }
    core_extensions {
        Address => {
            name: "address",
            items {
                AddressOffset => intrinsic AddressOffset, method_item!(
                    "offset", &[],
                    &[parameter("offset", U32, "The unsigned field offset.")],
                    ADDRESS, PURE, Availability::Everywhere,
                    "Adds a field offset to an address.",
                    "The offset is widened to the target address width.",
                    ADDRESS_OFFSET_EXAMPLE
                ),
                AddressAdd => intrinsic AddressAdd, method_item!(
                    "add", &[],
                    &[parameter("offset", U64, "The full-width address offset.")],
                    ADDRESS, PURE, Availability::Everywhere,
                    "Adds a full-width offset to an address.",
                    "This is useful for offsets that are already represented as u64.",
                    ADDRESS_ADD_EXAMPLE
                )
            }
        }
    }
    namespaces {
        Process => {
            name: "process",
            path: &["process"],
            qualified: "process",
            summary: "Accesses the attached game process.",
            details: "Process operations discover modules, read memory, follow pointers, and scan signatures.",
            items {
                ProcessModule => intrinsic ProcessModule, function_item!(
                    "module",
                    &[literal_parameter("name", STRING, ParameterRule::StringLiteral, "The exact module name.")],
                    MODULE, PROCESS_SUSPEND, Availability::OnAttach,
                    "Waits for a process module.",
                    "Suspends attachment until both module address and size are available.",
                    PROCESS_MODULE_EXAMPLE
                ),
                ProcessRead => intrinsic ProcessRead, typed_function_item!(
                    "read", "T", MEMORY_PARAMETER,
                    &[parameter("address", ADDRESS, "The target address to read.")],
                    T_RESULT, PROCESS, Availability::Everywhere,
                    "Reads a fixed-layout value from process memory.",
                    "The expected MemoryReadable type selects a fixed-size primitive or record layout. A synchronous read returns T!; retry polls until a value is available and yields T. Use a suffix such as process.read.i32 when context cannot determine a primitive type.",
                    PROCESS_READ_EXAMPLE
                ),
                ProcessFollow => intrinsic ProcessFollow, function_item!(
                    "follow",
                    &[
                        parameter("base", ADDRESS, "The initial address."),
                        parameter("offsets", U64_ARRAY, "Pointer offsets to follow.")
                    ],
                    ADDRESS_RESULT, PROCESS, Availability::Everywhere,
                    "Follows a pointer path.",
                    "Each intermediate address is read as a 64-bit target pointer. A failed or null pointer read returns an error; use retry in onAttach to wait for success.",
                    PROCESS_FOLLOW_EXAMPLE
                ),
                ProcessScan => intrinsic ProcessScan, function_item!(
                    "scan",
                    &[
                        parameter("address", ADDRESS, "The beginning of the range."),
                        parameter("size", U64, "The number of bytes to scan."),
                        literal_parameter("signature", SIGNATURE, ParameterRule::SignatureLiteral, "The compile-time signature pattern.")
                    ],
                    ADDRESS, PROCESS_SUSPEND, Availability::OnAttach,
                    "Scans a process-memory range.",
                    "Suspends until the signature is found in the requested range.",
                    PROCESS_SCAN_EXAMPLE
                ),
                ProcessReadRelative32 => intrinsic ProcessReadRelative32, function_item!(
                    "readRelative32",
                    &[parameter("address", ADDRESS, "The address of a signed relative displacement.")],
                    ADDRESS_RESULT, PROCESS, Availability::Everywhere,
                    "Resolves a 32-bit relative address.",
                    "Reads a signed displacement and adds it to the address following the displacement. A failed or null target returns an error; use retry in onAttach to wait for success.",
                    PROCESS_RELATIVE_EXAMPLE
                )
            }
        },
        ProcessRead => {
            name: "read",
            path: &["process", "read"],
            qualified: "process.read",
            summary: "Reads typed values from process memory.",
            details: "The expected type or an explicit suffix selects the process-memory layout.",
            items {
                ProcessReadManagedString => intrinsic ProcessReadManagedString, function_item!(
                    "managedString",
                    &[
                        parameter("address", ADDRESS, "The managed string object address."),
                        parameter("maxUtf16Units", U32, "The maximum UTF-16 code units to decode.")
                    ],
                    STRING_RESULT, PROCESS, Availability::Everywhere,
                    "Reads a Unity managed string.",
                    "The bounded UTF-16 payload is decoded into an immutable SplitScript string. Memory-access failure returns an error; malformed surrogate sequences decode as the replacement character.",
                    MANAGED_STRING_EXAMPLE
                )
            }
        },
        Timer => {
            name: "timer",
            path: &["timer"],
            qualified: "timer",
            summary: "Reads information from the LiveSplit timer.",
            details: "Timer operations expose runtime state used by autosplitter decisions.",
            items {
                TimerState => intrinsic TimerState, function_item!(
                    "state", &[], TIMER_STATE, TIMER_READ, Availability::Everywhere,
                    "Returns the current timer state.",
                    "Host states are converted to NotRunning, Running, Paused, Ended, or Unknown at the ABI boundary.",
                    TIMER_STATE_EXAMPLE
                )
            }
        },
        Unity => {
            name: "Unity",
            path: &["Unity"],
            qualified: "Unity",
            summary: "Discovers and inspects Unity runtimes.",
            details: "Unity operations attach to IL2CPP metadata and produce typed images, classes, and fields.",
            items {
                UnityIl2Cpp => intrinsic UnityIl2Cpp, function_item!(
                    "il2cpp",
                    &[parameter("version", U32, "The Unity metadata layout version.")],
                    UNITY_MODULE, PROCESS_SUSPEND, Availability::OnAttach,
                    "Discovers an IL2CPP runtime.",
                    "Suspends until GameAssembly and the IL2CPP metadata structures are available.",
                    UNITY_IL2CPP_EXAMPLE
                )
            }
        }
    }
    types {
        String => {
            name: "String",
            kind: Intrinsic,
            capabilities: EQUATABLE_INTERPOLATABLE,
            representation: RuntimeRepresentation::GcArray {
                element: CoreTypeId::U8,
                mutable: true,
                nullable: true,
            },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Stores immutable UTF-8 text.",
            details: "String literals, interpolation, process decoders, and timer variables use garbage-collected strings.",
            fields {}
            variants {}
            items {
                StringLength => intrinsic StringLength, function_item!(
                    "length",
                    &[parameter("value", STRING, "The string whose UTF-8 byte length is returned.")],
                    U32, PURE, Availability::Everywhere,
                    "Returns a string's UTF-8 byte length.",
                    "The result counts UTF-8 bytes in the current string representation.",
                    STRING_LENGTH_EXAMPLE
                ),
                StringConcat => intrinsic StringConcat, function_item!(
                    "concat",
                    &[parameter("values", STRING_ARRAY, "The strings to concatenate in order.")],
                    STRING, ALLOCATES, Availability::Everywhere,
                    "Concatenates an array of strings.",
                    "A new WebAssembly GC string is allocated.",
                    STRING_CONCAT_EXAMPLE
                )
            }
        },
        Signature => {
            name: "Signature",
            kind: Intrinsic,
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
            summary: "Stores a compiled process-memory signature.",
            details: "Signature values are created by signature literals and consumed by scanning operations.",
            fields {}
            variants {}
            items {}
        },
        Duration => {
            name: "Duration",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct {
                nullable: false,
            },
            value_usage: ValueUsage {
                record_field: true,
                enum_payload: false,
                state_field: false,
                local_variable: false,
                global_variable: false,
            },
            summary: "Represents a precise span of time.",
            details: "Durations carry whole seconds and nanoseconds and are used for LiveSplit game time.",
            fields {
                DurationSeconds => {
                    name: "seconds",
                    ty: DeclaredTypeRef::Core(CoreTypeId::I64),
                    visibility: RuntimePrivate,
                    docs: "Stores the whole-second component.",
                },
                DurationNanoseconds => {
                    name: "nanoseconds",
                    ty: DeclaredTypeRef::Core(CoreTypeId::I32),
                    visibility: RuntimePrivate,
                    docs: "Stores the fractional nanosecond component.",
                }
            }
            variants {}
            items {
                DurationFromFrames => intrinsic DurationFromFrames, function_item!(
                    "fromFrames",
                    &[
                        parameter("frames", I64, "The elapsed frame count."),
                        parameter("framesPerSecond", I64, "The frame rate.")
                    ],
                    DURATION, PURE, Availability::Everywhere,
                    "Constructs a duration from frames.",
                    "The conversion preserves whole seconds and nanoseconds.",
                    DURATION_FRAMES_EXAMPLE
                ),
                DurationFromParts => intrinsic DurationFromParts, function_item!(
                    "fromParts",
                    &[
                        parameter("seconds", I64, "Whole seconds."),
                        parameter("nanoseconds", I32, "The fractional nanoseconds.")
                    ],
                    DURATION, PURE, Availability::Everywhere,
                    "Constructs a duration from seconds and nanoseconds.",
                    "The two components become the runtime duration representation.",
                    DURATION_PARTS_EXAMPLE
                ),
                DurationFromSeconds => intrinsic DurationSaturatingSecondsF32, function_item!(
                    "fromSeconds",
                    &[parameter("seconds", F32, "Floating-point seconds.")],
                    DURATION, PURE, Availability::Everywhere,
                    "Constructs a duration from floating-point seconds.",
                    "Finite values are converted to the runtime duration representation; values outside its range are safely clamped.",
                    DURATION_SECONDS_EXAMPLE
                )
            }
        },
        Module => {
            name: "Module",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Describes a module loaded in the attached process.",
            details: "A module exposes its base address and mapped size for bounded memory discovery.",
            fields {
                ModuleAddress => {
                    name: "address",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: Public,
                    docs: "Returns the module base address.",
                },
                ModuleSize => {
                    name: "size",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U64),
                    visibility: Public,
                    docs: "Returns the mapped module size.",
                }
            }
            variants {}
            items {
                ModuleScan => intrinsic ModuleScan, method_item!(
                    "scan", &[],
                    &[literal_parameter("signature", SIGNATURE, ParameterRule::SignatureLiteral, "The compile-time signature pattern.")],
                    ADDRESS, PROCESS_SUSPEND, Availability::OnAttach,
                    "Scans a module for a signature.",
                    "The module's address and size define the scanned range.",
                    MODULE_SCAN_EXAMPLE
                )
            }
        },
        TimerState => {
            name: "TimerState",
            kind: Enum,
            capabilities: EQUATABLE,
            representation: RuntimeRepresentation::Enum { nullable: true },
            value_usage: ValueUsage {
                global_variable: true,
                ..ORDINARY_LOCAL_VALUE
            },
            summary: "Describes the current LiveSplit timer state.",
            details: "Timer state is an exhaustive enum returned by timer.state().",
            fields {}
            variants {
                TimerStateNotRunning => {
                    name: "NotRunning",
                    docs: "The timer has not started.",
                },
                TimerStateRunning => {
                    name: "Running",
                    docs: "The timer is running.",
                },
                TimerStatePaused => {
                    name: "Paused",
                    docs: "The timer is paused.",
                },
                TimerStateEnded => {
                    name: "Ended",
                    docs: "The timer has ended.",
                },
                TimerStateUnknown => {
                    name: "Unknown",
                    docs: "The host returned an unknown timer state.",
                }
            }
            items {}
        },
        UnityModule => {
            name: "UnityModule",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Describes an attached Unity IL2CPP runtime.",
            details: "The runtime stores resolved metadata roots, its version, and pointer size.",
            fields {
                UnityModuleAssemblies => {
                    name: "assemblies",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: Public,
                    docs: "Returns the IL2CPP assemblies metadata address.",
                },
                UnityModuleTypeInfoTable => {
                    name: "typeInfoTable",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: Public,
                    docs: "Returns the IL2CPP type-information table address.",
                },
                UnityModuleVersion => {
                    name: "version",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: Public,
                    docs: "Returns the detected Unity metadata version.",
                },
                UnityModulePointerSize => {
                    name: "pointerSize",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: Public,
                    docs: "Returns the attached process pointer size.",
                }
            }
            variants {}
            items {
                UnityModuleImage => intrinsic UnityModuleImage, method_item!(
                    "image", &[],
                    &[parameter("name", STRING, "The managed assembly name.")],
                    UNITY_IMAGE, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds an IL2CPP image.",
                    "Suspends until the named image is discoverable.",
                    UNITY_IMAGE_EXAMPLE
                )
            }
        },
        UnityImage => {
            name: "UnityImage",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Describes a Unity assembly image.",
            details: "An image retains its owning Unity runtime for subsequent class lookup.",
            fields {
                UnityImageAddress => {
                    name: "address",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: Public,
                    docs: "Returns the Unity image address.",
                },
                UnityImageModule => {
                    name: "module",
                    ty: DeclaredTypeRef::Standard(StdlibTypeId::UnityModule),
                    visibility: RuntimePrivate,
                    docs: "Retains the owning Unity runtime.",
                }
            }
            variants {}
            items {
                UnityImageClass => intrinsic UnityImageClass, method_item!(
                    "class", &[],
                    &[parameter("name", STRING, "The managed class name.")],
                    UNITY_CLASS, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds a class in an IL2CPP image.",
                    "Suspends until the named class is discoverable.",
                    UNITY_CLASS_EXAMPLE
                )
            }
        },
        UnityClass => {
            name: "UnityClass",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Describes a Unity runtime class.",
            details: "A class retains its owning Unity runtime for field and static-data discovery.",
            fields {
                UnityClassAddress => {
                    name: "address",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: Public,
                    docs: "Returns the Unity class address.",
                },
                UnityClassModule => {
                    name: "module",
                    ty: DeclaredTypeRef::Standard(StdlibTypeId::UnityModule),
                    visibility: RuntimePrivate,
                    docs: "Retains the owning Unity runtime.",
                }
            }
            variants {}
            items {
                UnityClassField => intrinsic UnityClassField, method_item!(
                    "field", &[],
                    &[parameter("name", STRING, "The managed field name.")],
                    U32, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds a managed field offset.",
                    "Searches the class hierarchy and recognizes backing fields.",
                    UNITY_FIELD_EXAMPLE
                ),
                UnityClassFieldAny => intrinsic UnityClassFieldAny, method_item!(
                    "fieldAny", &[],
                    &[parameter("names", STRING_ARRAY, "Candidate field names in priority order.")],
                    UNITY_FIELD, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds the first matching field.",
                    "Returns both the field offset and selected candidate index.",
                    UNITY_FIELD_ANY_EXAMPLE
                ),
                UnityClassStaticTable => intrinsic UnityClassStaticTable, method_item!(
                    "staticTable", &[], &[], ADDRESS, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds a class's static-field table.",
                    "Suspends until the static storage pointer is non-null.",
                    UNITY_STATIC_TABLE_EXAMPLE
                ),
                UnityClassStaticInstance => intrinsic UnityClassStaticInstance, method_item!(
                    "staticInstance", &[],
                    &[parameter("names", STRING_ARRAY, "Candidate singleton field names.")],
                    ADDRESS, PROCESS_SUSPEND, Availability::OnAttach,
                    "Finds a static singleton instance.",
                    "Combines field discovery, static-table lookup, and a non-null pointer read.",
                    UNITY_STATIC_INSTANCE_EXAMPLE
                )
            }
        },
        UnityField => {
            name: "UnityField",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Describes a Unity runtime field.",
            details: "A field exposes its byte offset and metadata index.",
            fields {
                UnityFieldOffset => {
                    name: "offset",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: Public,
                    docs: "Returns the instance-field byte offset.",
                },
                UnityFieldIndex => {
                    name: "index",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: Public,
                    docs: "Returns the metadata field index.",
                }
            }
            variants {}
            items {}
        },
        #[cfg(test)]
        CatalogRecordProbe => {
            name: "CatalogRecordProbe",
            kind: Struct,
            capabilities: &[StdlibCapabilityId::Equatable, StdlibCapabilityId::MemoryReadable],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Exercises the generic standard-library record pipeline.",
            details: "This declaration exists only in tests and deliberately has no intrinsic implementation.",
            fields {
                #[cfg(test)]
                CatalogRecordProbeValue => {
                    name: "value",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: Public,
                    docs: "Returns the probe value.",
                }
            }
            variants {}
            items {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchical_declarations_generate_the_complete_owner_graph() {
        let library = StandardLibrary::new();

        let unity_fields = library
            .fields_of(StdlibTypeId::UnityClass)
            .map(|field| (field.name, field.visibility))
            .collect::<Vec<_>>();
        assert_eq!(
            unity_fields,
            vec![
                ("address", FieldVisibility::Public),
                ("module", FieldVisibility::RuntimePrivate),
            ]
        );

        let unity_methods = library
            .items()
            .iter()
            .filter(|item| item.owner == StdlibOwner::Type(StdlibTypeId::UnityClass))
            .map(|item| (item.name, item.qualified_name))
            .collect::<Vec<_>>();
        assert_eq!(
            unity_methods,
            vec![
                ("field", "UnityClass.field"),
                ("fieldAny", "UnityClass.fieldAny"),
                ("staticTable", "UnityClass.staticTable"),
                ("staticInstance", "UnityClass.staticInstance"),
            ]
        );

        assert_eq!(
            library
                .variants_of(StdlibTypeId::TimerState)
                .map(|variant| variant.name)
                .collect::<Vec<_>>(),
            vec!["NotRunning", "Running", "Paused", "Ended", "Unknown"]
        );
        assert_eq!(
            library.item(StdlibItemId::DurationFromSeconds).owner,
            StdlibOwner::Type(StdlibTypeId::Duration)
        );
        assert_eq!(
            library.item(StdlibItemId::NumericClamp).owner,
            StdlibOwner::Capability(StdlibCapabilityId::Numeric)
        );
        assert_eq!(
            library.item(StdlibItemId::ArrayGet).owner,
            StdlibOwner::TypeConstructor(StdlibTypeConstructorId::Array)
        );
    }

    #[test]
    fn the_retired_parallel_authoring_registries_do_not_return() {
        let parent = include_str!("../stdlib.rs");
        let declarations = include_str!("declarations.rs");
        for retired in [
            "declare_standard_types!",
            "declare_standard_namespaces!",
            "declare_standard_fields!",
            "declare_standard_variants!",
            "declare_standard_items!",
        ] {
            assert!(
                !parent.contains(retired),
                "found retired registry `{retired}`"
            );
            assert!(
                !declarations.contains(retired),
                "found retired registry `{retired}`"
            );
        }
    }
}
