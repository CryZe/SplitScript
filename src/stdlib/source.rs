//! Single authored token stream for the bundled standard library.
//!
//! This module deliberately contains no schema, compiler, or backend types.
//! Independent consumers generate stable IDs and normalized declarations from
//! exactly the same hierarchy.

macro_rules! with_standard_library {
    ($consumer:ident) => {
        $consumer! {
    root {
        items {
            Print => {
                intrinsic: Print,
                kind: function,
                name: "print",
                type_parameters: &[],
                parameters: &[parameter("message", STRING, "The message to write to the runtime log.")],
                result: VOID,
                effects: RUNTIME_WRITE,
                availability: Availability::Everywhere,
                summary: "Prints a diagnostic message.",
                details: "The message is forwarded to the autosplitting runtime.",
                example: {
                    title: "Write to the runtime log",
                    source: "print(\"Attached to the game\")",
                    validation: BASIC_EXAMPLE,
                },
            },
            SetVariable => {
                intrinsic: TimerSetVariable,
                kind: function,
                name: "setVariable",
                type_parameters: &[],
                parameters: &[
                    parameter("name", STRING, "The variable name."),
                    parameter("value", STRING, "The displayed value.")
                ],
                result: VOID,
                effects: TIMER_WRITE,
                availability: Availability::Everywhere,
                summary: "Sets a LiveSplit custom variable.",
                details: "The value is visible to layouts that display autosplitter variables.",
                example: {
                    title: "Expose a layout variable",
                    source: "setVariable(\"Level\", levelName)",
                    validation: BASIC_EXAMPLE,
                },
            },
            SetTickRate => {
                intrinsic: RuntimeSetTickRate,
                kind: function,
                name: "setTickRate",
                type_parameters: &[],
                parameters: &[parameter("hz", F64, "The requested updates per second.")],
                result: VOID,
                effects: RUNTIME_WRITE,
                availability: Availability::Everywhere,
                summary: "Changes the autosplitter tick rate.",
                details: "The runtime applies the requested polling frequency.",
                example: {
                    title: "Poll more frequently",
                    source: "setTickRate(120.0)",
                    validation: BASIC_EXAMPLE,
                },
            },
            NextTick => {
                intrinsic: NextTick,
                kind: function,
                name: "nextTick",
                type_parameters: &[],
                parameters: &[],
                result: VOID,
                effects: NEXT_TICK,
                availability: Availability::OnAttach,
                summary: "Continues attachment on the next runtime update.",
                details: "Always suspends once. The continuation resumes on the following attached-process tick and is cancelled if that process closes first.",
                example: {
                    title: "Resume on the next update",
                    source: "await nextTick()",
                    validation: NEXT_TICK_EXAMPLE,
                },
            }
        }
    }
    state_providers {
        Gba => {
            name: "GBA",
            value_name: "gba",
            processes: &[
                "visualboyadvance-m.exe",
                "VisualBoyAdvance.exe",
                "mGBA.exe",
                "mGBA",
                "NO$GBA.EXE",
                "retroarch.exe",
                "EmuHawk.exe",
                "mednafen.exe"
            ],
            process_type: GbaEmulator,
            attachment: GbaAttach,
            direct_read: GbaEmulatorRead,
            summary: "Attaches to a supported Game Boy Advance emulator.",
            details: "The provider discovers emulated EWRAM and IWRAM and exposes original GBA hardware addresses through its read-only `gba` value.",
            example: {
                title: "Read GBA work RAM",
                source: "state GBA {\n    room: u8 at 0x03000010\n}",
                validation: GBA_EXAMPLE,
            }
        }
    }
    capabilities {
        Numeric => {
            name: "Numeric",
            behavior: Declared,
            receiver: T_REF,
            summary: "Supports ordered numeric operations.",
            details: "Numeric values provide minimum, maximum, and clamping operations while preserving their inferred type.",
            items {
                NumericMin => {
                    intrinsic: NumericMin,
                    kind: method,
                    name: "min",
                    type_parameters: NUMERIC_PARAMETER,
                    parameters: &[parameter("other", T_REF, "The other value to compare with the receiver.")],
                    result: T_REF,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Returns the smaller of two numeric values.",
                    details: "Both values have the same inferred numeric type and are evaluated once.",
                    example: {
                        title: "Keep the smaller value",
                        source: "let visibleStage = stage.min(7)",
                        validation: NUMERIC_EXAMPLE,
                    },
                },
                NumericMax => {
                    intrinsic: NumericMax,
                    kind: method,
                    name: "max",
                    type_parameters: NUMERIC_PARAMETER,
                    parameters: &[parameter("other", T_REF, "The other value to compare with the receiver.")],
                    result: T_REF,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Returns the larger of two numeric values.",
                    details: "Both values have the same inferred numeric type and are evaluated once.",
                    example: {
                        title: "Keep the larger value",
                        source: "let nonNegativeScore = score.max(0)",
                        validation: NUMERIC_EXAMPLE,
                    },
                },
                NumericClamp => {
                    intrinsic: NumericClamp,
                    kind: method,
                    name: "clamp",
                    type_parameters: NUMERIC_PARAMETER,
                    parameters: &[
                        parameter("minimum", T_REF, "The inclusive lower bound."),
                        parameter("maximum", T_REF, "The inclusive upper bound.")
                    ],
                    result: T_REF,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Restricts a numeric value to an inclusive range.",
                    details: "The receiver and bounds have one inferred numeric type and are evaluated once.",
                    example: {
                        title: "Restrict a value to a range",
                        source: "let visibleStage = stage.clamp(1, 7)",
                        validation: NUMERIC_EXAMPLE,
                    },
                }
            }
        },
        Integer => {
            name: "Integer", behavior: Declared, receiver: T_REF,
            summary: "Identifies integral numeric types.",
            details: "Integer operations and inference constraints accept only whole-number representations.",
            items {}
        },
        Signed => {
            name: "Signed", behavior: Declared, receiver: T_REF,
            summary: "Identifies signed numeric types.",
            details: "Signed values can represent numbers below zero.",
            items {}
        },
        Float => {
            name: "Float", behavior: Declared, receiver: T_REF,
            summary: "Identifies floating-point types.",
            details: "Floating-point constraints accept f32 and f64 values.",
            items {}
        },
        Equatable => {
            name: "Equatable", behavior: StructuralEquality, receiver: T_REF,
            summary: "Supports equality comparison.",
            details: "Equality is available for values whose representation can be compared safely.",
            items {}
        },
        StringCast => {
            name: "StringCast", behavior: Declared, receiver: T_REF,
            summary: "Supports explicit conversion to String.",
            details: "Values with this capability may be converted with an `as String` cast.",
            items {}
        },
        Interpolatable => {
            name: "Interpolatable", behavior: Declared, receiver: T_REF,
            summary: "Supports insertion into interpolated strings.",
            details: "Interpolated strings convert values with this capability into their textual representation.",
            items {}
        },
        MemoryReadable => {
            name: "MemoryReadable", behavior: StructuralMemoryLayout, receiver: T_REF,
            summary: "Supports deserialization from process memory.",
            details: "Primitive and record layouts with this capability can be selected by `process.read`.",
            items {}
        }
    }
    type_constructors {
        Array => {
            name: "Array",
            parameters: &["T"],
            receiver: T_ARRAY,
            summary: "Stores an indexed sequence of values.",
            details: "Arrays use WebAssembly GC storage and expose generic length, access, and mutation operations.",
            items {
                ArrayLength => {
                    intrinsic: ArrayLength,
                    kind: method,
                    name: "length",
                    type_parameters: UNCONSTRAINED_T,
                    parameters: &[],
                    result: U32,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Returns the number of array elements.",
                    details: "The result is the WebAssembly GC array length.",
                    example: {
                        title: "Count array elements",
                        source: "let fieldCount = fieldNames.length()",
                        validation: ARRAY_EXAMPLE,
                    },
                },
                ArrayGet => {
                    intrinsic: ArrayGet,
                    kind: method,
                    name: "get",
                    type_parameters: UNCONSTRAINED_T,
                    parameters: &[parameter("index", U32, "The zero-based element index.")],
                    result: T_REF,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Returns an array element.",
                    details: "Indexing uses WebAssembly's bounds checks.",
                    example: {
                        title: "Read an array element",
                        source: "let firstField = fieldNames.get(0)",
                        validation: ARRAY_EXAMPLE,
                    },
                },
                ArraySet => {
                    intrinsic: ArraySet,
                    kind: method,
                    name: "set",
                    type_parameters: UNCONSTRAINED_T,
                    parameters: &[
                        parameter("index", U32, "The zero-based element index."),
                        parameter("value", T_REF, "The new element value.")
                    ],
                    result: VOID,
                    effects: MUTATES_VALUE,
                    availability: Availability::Everywhere,
                    summary: "Updates an array element.",
                    details: "The array is evaluated once and updated in place.",
                    example: {
                        title: "Replace an array element",
                        source: "fieldNames.set(0, \"health\")",
                        validation: ARRAY_EXAMPLE,
                    },
                }
            }
        },
        Option => {
            name: "Option",
            parameters: &["T"],
            receiver: T_OPTION,
            summary: "Represents a value that may be absent.",
            details: "The postfix `?` type syntax constructs an Option value.",
            items {}
        },
        Result => {
            name: "Result",
            parameters: &["T"],
            receiver: T_RESULT,
            summary: "Represents a value or an error.",
            details: "The postfix `!` type syntax constructs a Result value.",
            items {}
        }
    }
    core_extensions {
        Address => {
            name: "address",
            items {
                AddressOffset => {
                    intrinsic: AddressOffset,
                    kind: method,
                    name: "offset",
                    type_parameters: &[],
                    parameters: &[parameter("offset", U32, "The unsigned field offset.")],
                    result: ADDRESS,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Adds a field offset to an address.",
                    details: "The offset is widened to the target address width.",
                    example: {
                        title: "Add a field offset",
                        source: "let healthAddress = player.offset(0x20)",
                        validation: ADDRESS_EXAMPLE,
                    },
                },
                AddressAdd => {
                    intrinsic: AddressAdd,
                    kind: method,
                    name: "add",
                    type_parameters: &[],
                    parameters: &[parameter("offset", U64, "The full-width address offset.")],
                    result: ADDRESS,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Adds a full-width offset to an address.",
                    details: "This is useful for offsets that are already represented as u64.",
                    example: {
                        title: "Add a full-width offset",
                        source: "let target = module.address.add(sectionOffset)",
                        validation: ADDRESS_EXAMPLE,
                    },
                }
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
                ProcessModule => {
                    intrinsic: ProcessModule,
                    kind: function,
                    name: "module",
                    type_parameters: &[],
                    parameters: &[literal_parameter("name", STRING, ParameterRule::StringLiteral, "The exact module name.")],
                    result: MODULE,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Waits for a process module.",
                    details: "Suspends attachment until both module address and size are available.",
                    example: {
                        title: "Wait for a module",
                        source: "let gameAssembly = await process.module(\"GameAssembly.dll\")",
                        validation: PROCESS_EXAMPLE,
                    },
                },
                ProcessRead => {
                    intrinsic: ProcessRead,
                    kind: typed_function("T"),
                    name: "read",
                    type_parameters: MEMORY_PARAMETER,
                    parameters: &[parameter("address", ADDRESS, "The target address to read.")],
                    result: T_RESULT,
                    effects: PROCESS,
                    availability: Availability::Everywhere,
                    summary: "Reads a fixed-layout value from process memory.",
                    details: "The expected MemoryReadable type selects a fixed-size primitive or record layout. A synchronous read returns T!; retry polls until a value is available and yields T. Use a suffix such as process.read.i32 when context cannot determine a primitive type.",
                    example: {
                        title: "Read a typed value",
                        source: "let health = process.read.i32(player.offset(0x20)) else 0",
                        validation: PROCESS_EXAMPLE,
                    },
                },
                ProcessFollow => {
                    intrinsic: ProcessFollow,
                    kind: function,
                    name: "follow",
                    type_parameters: &[],
                    parameters: &[
                        parameter("base", ADDRESS, "The initial address."),
                        parameter("offsets", U64_ARRAY, "Pointer offsets to follow.")
                    ],
                    result: ADDRESS_RESULT,
                    effects: PROCESS,
                    availability: Availability::Everywhere,
                    summary: "Follows a pointer path.",
                    details: "Each intermediate address is read as a 64-bit target pointer. A failed or null pointer read returns an error; use retry in onAttach to wait for success.",
                    example: {
                        title: "Follow a pointer path",
                        source: "let player = retry process.follow(module.address, [0x100, 0x20])",
                        validation: PROCESS_EXAMPLE,
                    },
                },
                ProcessScan => {
                    intrinsic: ProcessScan,
                    kind: function,
                    name: "scan",
                    type_parameters: &[],
                    parameters: &[
                        parameter("address", ADDRESS, "The beginning of the range."),
                        parameter("size", U64, "The number of bytes to scan."),
                        literal_parameter("signature", SIGNATURE, ParameterRule::SignatureLiteral, "The compile-time signature pattern.")
                    ],
                    result: ADDRESS,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Scans a process-memory range.",
                    details: "Suspends until the signature is found in the requested range.",
                    example: {
                        title: "Scan a memory range",
                        source: "let marker = await process.scan(module.address, module.size, sig\"48 8B ?? 89\")",
                        validation: PROCESS_EXAMPLE,
                    },
                },
                ProcessReadRelative32 => {
                    intrinsic: ProcessReadRelative32,
                    kind: function,
                    name: "readRelative32",
                    type_parameters: &[],
                    parameters: &[parameter("address", ADDRESS, "The address of a signed relative displacement.")],
                    result: ADDRESS_RESULT,
                    effects: PROCESS,
                    availability: Availability::Everywhere,
                    summary: "Resolves a 32-bit relative address.",
                    details: "Reads a signed displacement and adds it to the address following the displacement. A failed or null target returns an error; use retry in onAttach to wait for success.",
                    example: {
                        title: "Resolve a relative target",
                        source: "let target = retry process.readRelative32(instruction.offset(3))",
                        validation: PROCESS_EXAMPLE,
                    },
                }
            }
        },
        ProcessRead => {
            name: "read",
            path: &["process", "read"],
            qualified: "process.read",
            summary: "Reads typed values from process memory.",
            details: "The expected type or an explicit suffix selects the process-memory layout.",
            items {
                ProcessReadManagedString => {
                    intrinsic: ProcessReadManagedString,
                    kind: function,
                    name: "managedString",
                    type_parameters: &[],
                    parameters: &[
                        parameter("address", ADDRESS, "The managed string object address."),
                        parameter("maxUtf16Units", U32, "The maximum UTF-16 code units to decode.")
                    ],
                    result: STRING_RESULT,
                    effects: PROCESS,
                    availability: Availability::Everywhere,
                    summary: "Reads a Unity managed string.",
                    details: "The bounded UTF-16 payload is decoded into an immutable SplitScript string. Memory-access failure returns an error; malformed surrogate sequences decode as the replacement character.",
                    example: {
                        title: "Read a Unity string",
                        source: "let scene = process.read.managedString(sceneAddress, 64) else \"Unknown\"",
                        validation: PROCESS_EXAMPLE,
                    },
                }
            }
        },
        Timer => {
            name: "timer",
            path: &["timer"],
            qualified: "timer",
            summary: "Reads information from the LiveSplit timer.",
            details: "Timer operations expose runtime state used by autosplitter decisions.",
            items {
                TimerState => {
                    intrinsic: TimerState,
                    kind: function,
                    name: "state",
                    type_parameters: &[],
                    parameters: &[],
                    result: TIMER_STATE,
                    effects: TIMER_READ,
                    availability: Availability::Everywhere,
                    summary: "Returns the current timer state.",
                    details: "Host states are converted to NotRunning, Running, Paused, Ended, or Unknown at the ABI boundary.",
                    example: {
                        title: "Check whether the timer is running",
                        source: "let isRunning = timer.state() == TimerState.Running",
                        validation: BASIC_EXAMPLE,
                    },
                }
            }
        },
        Unity => {
            name: "Unity",
            path: &["Unity"],
            qualified: "Unity",
            summary: "Discovers and inspects Unity runtimes.",
            details: "Unity operations attach to IL2CPP metadata and produce typed images, classes, and fields.",
            items {
                UnityIl2Cpp => {
                    intrinsic: UnityIl2Cpp,
                    kind: function,
                    name: "il2cpp",
                    type_parameters: &[],
                    parameters: &[parameter("version", U32, "The Unity metadata layout version.")],
                    result: UNITY_MODULE,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Discovers an IL2CPP runtime.",
                    details: "Suspends until GameAssembly and the IL2CPP metadata structures are available.",
                    example: {
                        title: "Discover IL2CPP metadata",
                        source: "let unity = await Unity.il2cpp(2020)",
                        validation: UNITY_EXAMPLE,
                    },
                }
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
                StringLength => {
                    intrinsic: StringLength,
                    kind: function,
                    name: "length",
                    type_parameters: &[],
                    parameters: &[parameter("value", STRING, "The string whose UTF-8 byte length is returned.")],
                    result: U32,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Returns a string's UTF-8 byte length.",
                    details: "The result counts UTF-8 bytes in the current string representation.",
                    example: {
                        title: "Measure UTF-8 text",
                        source: "let byteLength = String.length(levelName)",
                        validation: BASIC_EXAMPLE,
                    },
                },
                StringConcat => {
                    intrinsic: StringConcat,
                    kind: function,
                    name: "concat",
                    type_parameters: &[],
                    parameters: &[parameter("values", STRING_ARRAY, "The strings to concatenate in order.")],
                    result: STRING,
                    effects: ALLOCATES,
                    availability: Availability::Everywhere,
                    summary: "Concatenates an array of strings.",
                    details: "A new WebAssembly GC string is allocated.",
                    example: {
                        title: "Join strings",
                        source: "let label = String.concat([\"Stage \", stageName])",
                        validation: BASIC_EXAMPLE,
                    },
                }
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
                DurationFromFrames => {
                    intrinsic: DurationFromFrames,
                    kind: function,
                    name: "fromFrames",
                    type_parameters: &[],
                    parameters: &[
                        parameter("frames", I64, "The elapsed frame count."),
                        parameter("framesPerSecond", I64, "The frame rate.")
                    ],
                    result: DURATION,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Constructs a duration from frames.",
                    details: "The conversion preserves whole seconds and nanoseconds.",
                    example: {
                        title: "Convert frames to game time",
                        source: "return Duration.fromFrames(frameCount, 60)",
                        validation: DURATION_EXAMPLE,
                    },
                },
                DurationFromParts => {
                    intrinsic: DurationFromParts,
                    kind: function,
                    name: "fromParts",
                    type_parameters: &[],
                    parameters: &[
                        parameter("seconds", I64, "Whole seconds."),
                        parameter("nanoseconds", I32, "The fractional nanoseconds.")
                    ],
                    result: DURATION,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Constructs a duration from seconds and nanoseconds.",
                    details: "The two components become the runtime duration representation.",
                    example: {
                        title: "Construct an exact duration",
                        source: "return Duration.fromParts(seconds, nanoseconds)",
                        validation: DURATION_EXAMPLE,
                    },
                },
                DurationFromSeconds => {
                    intrinsic: DurationSaturatingSecondsF32,
                    kind: function,
                    name: "fromSeconds",
                    type_parameters: &[],
                    parameters: &[parameter("seconds", F32, "Floating-point seconds.")],
                    result: DURATION,
                    effects: PURE,
                    availability: Availability::Everywhere,
                    summary: "Constructs a duration from floating-point seconds.",
                    details: "Finite values are converted to the runtime duration representation; values outside its range are safely clamped.",
                    example: {
                        title: "Convert seconds to game time",
                        source: "return Duration.fromSeconds(elapsedSeconds)",
                        validation: DURATION_EXAMPLE,
                    },
                }
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
                ModuleScan => {
                    intrinsic: ModuleScan,
                    kind: method,
                    name: "scan",
                    type_parameters: &[],
                    parameters: &[literal_parameter("signature", SIGNATURE, ParameterRule::SignatureLiteral, "The compile-time signature pattern.")],
                    result: ADDRESS,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Scans a module for a signature.",
                    details: "The module's address and size define the scanned range.",
                    example: {
                        title: "Scan an entire module",
                        source: "let marker = await gameAssembly.scan(sig\"48 8B ?? 89\")",
                        validation: PROCESS_EXAMPLE,
                    },
                }
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
                UnityModuleImage => {
                    intrinsic: UnityModuleImage,
                    kind: method,
                    name: "image",
                    type_parameters: &[],
                    parameters: &[parameter("name", STRING, "The managed assembly name.")],
                    result: UNITY_IMAGE,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds an IL2CPP image.",
                    details: "Suspends until the named image is discoverable.",
                    example: {
                        title: "Find a managed assembly",
                        source: "let image = await unity.image(\"Assembly-CSharp\")",
                        validation: UNITY_EXAMPLE,
                    },
                }
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
                UnityImageClass => {
                    intrinsic: UnityImageClass,
                    kind: method,
                    name: "class",
                    type_parameters: &[],
                    parameters: &[parameter("name", STRING, "The managed class name.")],
                    result: UNITY_CLASS,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds a class in an IL2CPP image.",
                    details: "Suspends until the named class is discoverable.",
                    example: {
                        title: "Find a managed class",
                        source: "let gameManager = await image.class(\"GameManager\")",
                        validation: UNITY_EXAMPLE,
                    },
                }
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
                UnityClassField => {
                    intrinsic: UnityClassField,
                    kind: method,
                    name: "field",
                    type_parameters: &[],
                    parameters: &[parameter("name", STRING, "The managed field name.")],
                    result: U32,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds a managed field offset.",
                    details: "Searches the class hierarchy and recognizes backing fields.",
                    example: {
                        title: "Find a field offset",
                        source: "let healthOffset = await gameManager.field(\"health\")",
                        validation: UNITY_EXAMPLE,
                    },
                },
                UnityClassFieldAny => {
                    intrinsic: UnityClassFieldAny,
                    kind: method,
                    name: "fieldAny",
                    type_parameters: &[],
                    parameters: &[parameter("names", STRING_ARRAY, "Candidate field names in priority order.")],
                    result: UNITY_FIELD,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds the first matching field.",
                    details: "Returns both the field offset and selected candidate index.",
                    example: {
                        title: "Try multiple field names",
                        source: "let levelField = await gameManager.fieldAny([\"currentLevel\", \"level\"])",
                        validation: UNITY_EXAMPLE,
                    },
                },
                UnityClassStaticTable => {
                    intrinsic: UnityClassStaticTable,
                    kind: method,
                    name: "staticTable",
                    type_parameters: &[],
                    parameters: &[],
                    result: ADDRESS,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds a class's static-field table.",
                    details: "Suspends until the static storage pointer is non-null.",
                    example: {
                        title: "Find static storage",
                        source: "let staticTable = await gameManager.staticTable()",
                        validation: UNITY_EXAMPLE,
                    },
                },
                UnityClassStaticInstance => {
                    intrinsic: UnityClassStaticInstance,
                    kind: method,
                    name: "staticInstance",
                    type_parameters: &[],
                    parameters: &[parameter("names", STRING_ARRAY, "Candidate singleton field names.")],
                    result: ADDRESS,
                    effects: PROCESS_SUSPEND,
                    availability: Availability::OnAttach,
                    summary: "Finds a static singleton instance.",
                    details: "Combines field discovery, static-table lookup, and a non-null pointer read.",
                    example: {
                        title: "Find a singleton instance",
                        source: "let instance = await gameManager.staticInstance([\"Instance\", \"_instance\"])",
                        validation: UNITY_EXAMPLE,
                    },
                }
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
        GbaEmulator => {
            name: "GbaEmulator",
            kind: Struct,
            capabilities: &[],
            representation: RuntimeRepresentation::GcStruct { nullable: true },
            value_usage: ORDINARY_LOCAL_VALUE,
            summary: "Retains a discovered GBA emulator memory mapping.",
            details: "The runtime-private mapping identifies the emulator backend and translates EWRAM and IWRAM addresses before each read.",
            fields {
                GbaEmulatorBackend => {
                    name: "backend",
                    ty: DeclaredTypeRef::Core(CoreTypeId::U32),
                    visibility: RuntimePrivate,
                    docs: "Identifies the emulator-specific memory layout.",
                },
                GbaEmulatorEwram => {
                    name: "ewram",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: RuntimePrivate,
                    docs: "Stores the discovered EWRAM base when it is stable.",
                },
                GbaEmulatorIwram => {
                    name: "iwram",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: RuntimePrivate,
                    docs: "Stores the discovered IWRAM base when it is stable.",
                },
                GbaEmulatorAux1 => {
                    name: "aux1",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: RuntimePrivate,
                    docs: "Stores backend-specific discovery state.",
                },
                GbaEmulatorAux2 => {
                    name: "aux2",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: RuntimePrivate,
                    docs: "Stores backend-specific discovery state.",
                },
                GbaEmulatorAux3 => {
                    name: "aux3",
                    ty: DeclaredTypeRef::Core(CoreTypeId::Address),
                    visibility: RuntimePrivate,
                    docs: "Stores backend-specific discovery state.",
                }
            }
            variants {}
            items {
                GbaEmulatorRead => {
                    intrinsic: GbaEmulatorRead,
                    kind: method,
                    name: "read",
                    type_parameters: MEMORY_PARAMETER,
                    parameters: &[parameter("address", U32, "The original GBA hardware address.")],
                    result: T_RESULT,
                    effects: PROCESS,
                    availability: Availability::Everywhere,
                    summary: "Reads a typed value from emulated GBA memory.",
                    details: "The expected MemoryReadable type determines the byte layout. Valid reads lie entirely within EWRAM (0x02000000..0x02040000) or IWRAM (0x03000000..0x03008000).",
                    example: {
                        title: "Read an emulated value",
                        source: "let room: u8 = gba.read(0x03000010) else 0",
                        validation: GBA_EXAMPLE,
                    },
                }
            }
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
    };
}

pub(super) use with_standard_library;
