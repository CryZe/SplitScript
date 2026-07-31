//! Authored standard-library hierarchy and its generated flat symbol tables.
//!
//! This Rust declaration layer is intentionally an adapter: compiler and
//! tooling consumers depend on the normalized graph it emits, so a future
//! privileged SplitScript standard-library loader can replace this producer
//! without changing those consumers.

use crate::catalog::{Documentation, Example};

use super::{
    declarations::{
        CapabilityBehavior, CoreTypeId, DeclaredTypeRef, EQUATABLE, EQUATABLE_INTERPOLATABLE,
        FieldVisibility, ORDINARY_LOCAL_VALUE, RuntimeRepresentation, StdlibCapability,
        StdlibField, StdlibNamespace, StdlibOwner, StdlibStateProvider, StdlibSymbolId, StdlibType,
        StdlibTypeConstructor, StdlibTypeKind, StdlibVariant, ValueUsage,
    },
    ids::{
        IntrinsicId, StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId,
        StdlibStateProviderId, StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
    },
    schema::{
        Availability, Effect, EffectSet, Implementation, ItemKind, Parameter, ParameterRule,
        Signature, StdlibItem, TypeParameter, TypeRef,
    },
};

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
const T_REF: TypeRef = TypeRef::Parameter("T");
const T_ARGUMENTS: &[TypeRef] = &[T_REF];
const ADDRESS_ARGUMENTS: &[TypeRef] = &[ADDRESS];
const STRING_ARGUMENTS: &[TypeRef] = &[STRING];
const U64_ARGUMENTS: &[TypeRef] = &[U64];
const T_ARRAY: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: T_ARGUMENTS,
};
#[allow(dead_code)]
const T_OPTION: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Option,
    arguments: T_ARGUMENTS,
};
const T_RESULT: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: T_ARGUMENTS,
};
const ADDRESS_RESULT: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: ADDRESS_ARGUMENTS,
};
const STRING_RESULT: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Result,
    arguments: STRING_ARGUMENTS,
};
const STRING_ARRAY: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: STRING_ARGUMENTS,
};
const U64_ARRAY: TypeRef = TypeRef::Application {
    constructor: StdlibTypeConstructorId::Array,
    arguments: U64_ARGUMENTS,
};

const NUMERIC_PARAMETER: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[StdlibCapabilityId::Numeric],
}];
const MEMORY_PARAMETER: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[StdlibCapabilityId::MemoryReadable],
}];
const UNCONSTRAINED_T: &[TypeParameter] = &[TypeParameter {
    name: "T",
    constraints: &[],
}];

const PURE: EffectSet = EffectSet::one(Effect::Pure);
const ALLOCATES: EffectSet = EffectSet::one(Effect::Allocates);
const PROCESS: EffectSet =
    EffectSet::one(Effect::ReadsProcess).with(Effect::RequiresAttachedProcess);
const PROCESS_SUSPEND: EffectSet = PROCESS
    .with(Effect::Suspends)
    .with(Effect::CancelsOnProcessClose);
const NEXT_TICK: EffectSet = EffectSet::one(Effect::RequiresAttachedProcess)
    .with(Effect::Suspends)
    .with(Effect::CancelsOnProcessClose);
const TIMER_WRITE: EffectSet = EffectSet::one(Effect::WritesTimer);
const TIMER_READ: EffectSet = EffectSet::one(Effect::ReadsTimer);
const RUNTIME_WRITE: EffectSet = EffectSet::one(Effect::WritesRuntime);
const MUTATES_VALUE: EffectSet = EffectSet::one(Effect::MutatesValue);

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
const GBA_EXAMPLE: &str = r#"state GBA {
    room: u8 at 0x03000010
}"#;

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

macro_rules! stdlib_item_kind {
    (function, $receiver:expr) => {
        ItemKind::Function
    };
    (typed_function($type_parameter:literal), $receiver:expr) => {
        ItemKind::TypedFunction {
            type_parameter: $type_parameter,
        }
    };
    (method, $receiver:expr) => {
        ItemKind::Method {
            receiver: $receiver,
        }
    };
}

macro_rules! stdlib_receiver {
    ($default:expr) => {
        $default
    };
    ($default:expr, $receiver:expr) => {
        $receiver
    };
}

macro_rules! stdlib_qualified_name {
    (root, $prefix:literal, $name:literal) => {
        $name
    };
    (owned, $prefix:literal, $name:literal) => {
        concat!($prefix, ".", $name)
    };
}

macro_rules! stdlib_item {
    (@root $id:ident, $intrinsic:ident, $($declaration:tt)*) => {
        stdlib_item!(@emit root, $id, $intrinsic, StdlibOwner::Root, "", VOID, $($declaration)*)
    };
    (@owned $id:ident, $intrinsic:ident, $owner:expr, $prefix:literal, $receiver:expr,
     $($declaration:tt)*) => {
        stdlib_item!(@emit owned, $id, $intrinsic, $owner, $prefix, $receiver, $($declaration)*)
    };
    (@emit $qualification:tt, $id:ident, $intrinsic:ident, $owner:expr,
        $prefix:literal, $receiver:expr,
        $(receiver: $declared_receiver:expr,)?
        kind: $kind:ident $(($type_parameter:literal))?,
        name: $name:literal,
        type_parameters: $types:expr,
        parameters: $parameters:expr,
        result: $result:expr,
        effects: $effects:expr,
        availability: $availability:expr,
        summary: $summary:literal,
        details: $details:literal,
        example: {
            title: $example_title:literal,
            source: $example_source:literal,
            validation: $example_validation:expr $(,)?
        } $(,)?
    ) => {
        StdlibItem {
            id: StdlibItemId::$id,
            owner: $owner,
            name: $name,
            qualified_name: stdlib_qualified_name!($qualification, $prefix, $name),
            kind: stdlib_item_kind!(
                $kind $(($type_parameter))?,
                stdlib_receiver!($receiver $(, $declared_receiver)?)
            ),
            signature: Signature {
                type_parameters: $types,
                parameters: $parameters,
                result: $result,
            },
            effects: $effects,
            availability: $availability,
            deprecation: None,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    $example_title,
                    $example_source,
                    $example_validation,
                )],
                related: &[],
            },
            implementation: Implementation::Intrinsic(IntrinsicId::$intrinsic),
        }
    };
}

macro_rules! standard_library {
    (
        root {
            items { $($root_item_id:ident => {
                intrinsic: $root_intrinsic:ident, $($root_item:tt)*
            }),* $(,)? }
        }
        state_providers {
            $($state_provider_id:ident => {
                name: $state_provider_name:literal,
                value_name: $state_provider_value_name:literal,
                processes: $state_provider_processes:expr,
                process_type: $state_provider_type:ident,
                attachment: $state_provider_attachment:ident,
                direct_read: $state_provider_direct_read:ident,
                summary: $state_provider_summary:literal,
                details: $state_provider_details:literal,
                example: {
                    title: $state_provider_example_title:literal,
                    source: $state_provider_example_source:literal,
                    validation: $state_provider_example_validation:expr $(,)?
                } $(,)?
            }),* $(,)?
        }
        capabilities {
            $($capability_id:ident => {
                name: $capability_name:literal,
                behavior: $capability_behavior:ident,
                receiver: $capability_receiver:expr,
                summary: $capability_summary:literal,
                details: $capability_details:literal,
                items { $($capability_item_id:ident => {
                    intrinsic: $capability_intrinsic:ident, $($capability_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        type_constructors {
            $($constructor_id:ident => {
                name: $constructor_name:literal,
                parameters: $constructor_parameters:expr,
                receiver: $constructor_receiver:expr,
                summary: $constructor_summary:literal,
                details: $constructor_details:literal,
                items { $($constructor_item_id:ident => {
                    intrinsic: $constructor_intrinsic:ident, $($constructor_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        core_extensions {
            $($core_id:ident => {
                name: $core_name:literal,
                items { $($core_item_id:ident => {
                    intrinsic: $core_intrinsic:ident, $($core_item:tt)*
                }),* $(,)? }
            }),* $(,)?
        }
        namespaces {
            $($namespace_id:ident => {
                name: $namespace_name:literal,
                path: $namespace_path:expr,
                qualified: $namespace_qualified:literal,
                summary: $namespace_summary:literal,
                details: $namespace_details:literal,
                items { $($namespace_item_id:ident => {
                    intrinsic: $namespace_intrinsic:ident, $($namespace_item:tt)*
                }),* $(,)? }
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
                        $($type_item_id:ident => {
                            intrinsic: $type_intrinsic:ident, $($type_item:tt)*
                        }),* $(,)?
                    }
                }
            ),* $(,)?
        }
    ) => {
        pub(super) const STATE_PROVIDERS: &[StdlibStateProvider] = &[
            $(StdlibStateProvider {
                id: StdlibStateProviderId::$state_provider_id,
                name: $state_provider_name,
                value_name: $state_provider_value_name,
                processes: $state_provider_processes,
                process_type: StdlibTypeId::$state_provider_type,
                attachment: IntrinsicId::$state_provider_attachment,
                direct_read: StdlibItemId::$state_provider_direct_read,
                documentation: Documentation {
                    summary: $state_provider_summary,
                    details: $state_provider_details,
                    examples: &[Example::checked(
                        $state_provider_example_title,
                        $state_provider_example_source,
                        $state_provider_example_validation,
                    )],
                    related: &[],
                },
            }),*
        ];

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
                behavior: CapabilityBehavior::$capability_behavior,
                documentation: documentation($capability_summary, $capability_details),
            }),*
        ];

        pub(super) const TYPE_CONSTRUCTORS: &[StdlibTypeConstructor] = &[
            $(StdlibTypeConstructor {
                id: StdlibTypeConstructorId::$constructor_id,
                name: $constructor_name,
                parameters: $constructor_parameters,
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
            $(stdlib_item!(@root $root_item_id, $root_intrinsic, $($root_item)*),)*
            $($(stdlib_item!(@owned $capability_item_id, $capability_intrinsic,
                StdlibOwner::Capability(StdlibCapabilityId::$capability_id),
                $capability_name, $capability_receiver, $($capability_item)*),)*)*
            $($(stdlib_item!(@owned $constructor_item_id, $constructor_intrinsic,
                StdlibOwner::TypeConstructor(StdlibTypeConstructorId::$constructor_id),
                $constructor_name, $constructor_receiver, $($constructor_item)*),)*)*
            $($(stdlib_item!(@owned $core_item_id, $core_intrinsic,
                StdlibOwner::Core(CoreTypeId::$core_id), $core_name,
                TypeRef::Core(CoreTypeId::$core_id), $($core_item)*),)*)*
            $($(stdlib_item!(@owned $namespace_item_id, $namespace_intrinsic,
                StdlibOwner::Namespace(StdlibNamespaceId::$namespace_id),
                $namespace_qualified, VOID, $($namespace_item)*),)*)*
            $($(stdlib_item!(@owned $type_item_id, $type_intrinsic,
                StdlibOwner::Type(StdlibTypeId::$type_id), $type_name,
                TypeRef::Standard(StdlibTypeId::$type_id), $($type_item)*),)*)*
        ];
    };
}

super::source::with_standard_library!(standard_library);

#[cfg(test)]
mod tests {
    use crate::stdlib::StandardLibrary;

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
            .children_of(StdlibOwner::Type(StdlibTypeId::UnityClass))
            .filter_map(|symbol| match symbol {
                StdlibSymbolId::Item(item) => Some(library.item(item)),
                _ => None,
            })
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
        assert!(
            library
                .children_of(StdlibOwner::Namespace(StdlibNamespaceId::Process))
                .any(|child| child == StdlibSymbolId::Namespace(StdlibNamespaceId::ProcessRead))
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

    #[test]
    fn authored_source_ids_schema_and_normalized_data_have_one_way_dependencies() {
        let source = include_str!("source.rs");
        let ids = include_str!("ids.rs");
        let schema = include_str!("schema.rs");
        let declarations = include_str!("declarations.rs");
        let catalog = include_str!("catalog.rs");

        assert!(source.contains("macro_rules! with_standard_library"));
        assert!(ids.contains("with_standard_library!("));
        assert!(catalog.contains("with_standard_library!("));
        assert!(!ids.contains("super::declarations"));
        assert!(!ids.contains("super::catalog"));
        assert!(!schema.contains("super::catalog"));
        assert!(!schema.contains("super::graph"));
        assert!(!declarations.contains("super::catalog"));
        assert!(!declarations.contains("super::graph"));
        let retired_closed_type_id = ["pub enum ", "StdlibTypeId"].concat();
        assert!(!catalog.contains(&retired_closed_type_id));
        assert!(ids.contains("pub struct $name(u32)"));
    }

    #[test]
    fn callables_use_one_named_owned_declaration_grammar() {
        let source = include_str!("source.rs");
        let catalog = include_str!("catalog.rs");

        let retired_producers = [
            ["$function", "_item"].concat(),
            ["$typed_function", "_item"].concat(),
            ["$method", "_item"].concat(),
            ["macro_rules! function", "_item"].concat(),
            ["macro_rules! typed_function", "_item"].concat(),
            ["macro_rules! method", "_item"].concat(),
            ["doc_", "example!"].concat(),
        ];
        for retired in retired_producers {
            assert!(
                !source.contains(&retired) && !catalog.contains(&retired),
                "found retired positional callable producer `{retired}`"
            );
        }

        let callable_count = ITEMS.len();
        assert_eq!(source.matches("intrinsic:").count(), callable_count);
        assert_eq!(
            source.matches("example:").count(),
            callable_count + STATE_PROVIDERS.len()
        );
        for field in [
            "kind:",
            "type_parameters:",
            "parameters:",
            "result:",
            "effects:",
            "availability:",
            "summary:",
            "details:",
        ] {
            assert!(
                source.matches(field).count() >= callable_count,
                "missing `{field}`"
            );
        }
    }

    #[test]
    fn declaration_type_expressions_use_catalog_constructor_identities() {
        let library = StandardLibrary::new();

        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Array)
                .parameters,
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Option)
                .parameters,
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Result)
                .parameters,
            ["T"]
        );
        assert_eq!(
            library.render_signature(StdlibItemId::ProcessRead),
            "process.read(address: address) -> T! where T: MemoryReadable"
        );
        assert_eq!(
            library.render_signature(StdlibItemId::ArrayGet),
            "[T].get(index: u32) -> T where T"
        );
        assert!(library.validate().is_empty());
    }

    #[test]
    fn capability_bounds_and_behavior_are_catalog_facts() {
        let library = StandardLibrary::new();

        assert_eq!(
            library
                .item(StdlibItemId::NumericMin)
                .signature
                .type_parameters[0]
                .constraints,
            [StdlibCapabilityId::Numeric]
        );
        assert_eq!(
            library.capability(StdlibCapabilityId::Equatable).behavior,
            CapabilityBehavior::StructuralEquality
        );
        assert_eq!(
            library
                .capability(StdlibCapabilityId::MemoryReadable)
                .behavior,
            CapabilityBehavior::StructuralMemoryLayout
        );
        assert_eq!(
            library.capability(StdlibCapabilityId::Numeric).behavior,
            CapabilityBehavior::Declared
        );

        let schema = include_str!("schema.rs");
        let retired_constraint = ["enum Type", "Constraint"].concat();
        assert!(!schema.contains(&retired_constraint));
    }
}
