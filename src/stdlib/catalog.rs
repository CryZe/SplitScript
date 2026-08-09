//! Generated normalized standard-library catalog.
//!
//! The public surface is authored in `stdlib/standard.split`. This module keeps
//! only Rust-side construction helpers and compiler-checked example fixtures;
//! the build script includes the final typed declaration arrays generated from
//! that source.

use crate::catalog::{Documentation, Example};

use super::{
    declarations::{
        CapabilityBehavior, CoreTypeId, FieldVisibility, RuntimeRepresentation,
        StateProviderAttachment, StateProviderProcesses, StdlibCapability, StdlibField,
        StdlibNamespace, StdlibOwner, StdlibStateProvider, StdlibType, StdlibTypeConstructor,
        StdlibTypeKind, StdlibVariant, ValueUsage,
    },
    ids::{
        IntrinsicId, StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId,
        StdlibStateProviderId, StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
    },
    schema::{
        Implementation, ItemKind, Parameter, ParameterRule, Signature, StandardBinaryOperator,
        StdlibItem, TypeParameter, TypeRef,
    },
};

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
    setVariable("Length", joined.byteLength() as String)
    let state = timer.state()
    let running = timer.isRunning()
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
const FLOAT_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let sample: f32 = -1.25
    let whole: f64 = 2
    let magnitude = sample.abs()
    let lower = sample.floor()
    let upper = sample.ceil()
    let nearest = sample.round()
    let oneDecimalPlace = sample.roundTo(1)
    let finite = sample.isFinite()
    let notANumber = sample.isNaN()
}"#;
const ARRAY_EXAMPLE: &str = r#"state "game.exe" {}
whileAttached {
    let bytes = [0x48u8, 0u8]
    bytes.set(1, 0x8bu8)
    let first = bytes[0]
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
    let processMarker = await process.scanMemory(sig"54 49 4D 52 ?? ?? ?? ??")
    let object = retry process.follow(module.address, [0x100, 0x20])
    let health = retry process.read<i32>(object.offset(0x10))
    let target = retry process.readRelative32(marker.offset(3))
    let moduleTarget = retry module.readRelative32(0x400)
    let scene = retry process.readManagedString(target, 64)
    print(`{rangedMarker}:{processMarker}:{health}:{moduleTarget}:{scene}`)
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
const SETTINGS_EXAMPLE: &str = r#"state "game.exe" {}
settings {
    "Enabled" => enabled: true,
    "Split boss" => splitBoss key "split-boss": true
}
whileAttached {
    let direct = settings.enabled
    let enabled = settings.enabled("split-boss")
    let wasEnabled = oldSettings.enabled("split-boss")
}"#;
const GBA_EXAMPLE: &str = r#"state GBA {
    room: u8 at 0x03000010
}"#;

/// Supplies a complete compiler fixture for each focused documentation
/// snippet. This belongs to the compiler-side intrinsic trust boundary rather
/// than the source loader: adding an intrinsic must update its lowering,
/// contract, and documentation validation context together.
const fn validation_fixture(item: StdlibItemId) -> &'static str {
    match item {
        StdlibItemId::NextTick => NEXT_TICK_EXAMPLE,
        StdlibItemId::SettingsViewEnabled => SETTINGS_EXAMPLE,
        StdlibItemId::NumericAdd
        | StdlibItemId::NumericSubtract
        | StdlibItemId::NumericMin
        | StdlibItemId::NumericMax
        | StdlibItemId::NumericClamp => NUMERIC_EXAMPLE,
        StdlibItemId::FloatAbs
        | StdlibItemId::FloatFloor
        | StdlibItemId::FloatCeil
        | StdlibItemId::FloatRound
        | StdlibItemId::FloatRoundTo
        | StdlibItemId::FloatIsNaN
        | StdlibItemId::FloatIsFinite => FLOAT_EXAMPLE,
        StdlibItemId::ArrayLength | StdlibItemId::ArraySet => ARRAY_EXAMPLE,
        StdlibItemId::AddressOffset | StdlibItemId::AddressAdd => ADDRESS_EXAMPLE,
        StdlibItemId::ProcessModule
        | StdlibItemId::ProcessRead
        | StdlibItemId::ProcessFollow
        | StdlibItemId::ProcessScan
        | StdlibItemId::ProcessScanMemory
        | StdlibItemId::ProcessScanMemoryAny
        | StdlibItemId::ProcessReadRelative32
        | StdlibItemId::ProcessReadUtf8
        | StdlibItemId::ProcessReadUtf16Le
        | StdlibItemId::ProcessReadManagedString
        | StdlibItemId::ModuleScan
        | StdlibItemId::ModuleReadRelative32 => PROCESS_EXAMPLE,
        StdlibItemId::UnityIl2Cpp
        | StdlibItemId::UnityModuleImage
        | StdlibItemId::UnityImageClass
        | StdlibItemId::UnityClassField
        | StdlibItemId::UnityClassFieldAny
        | StdlibItemId::UnityClassStaticTable
        | StdlibItemId::UnityClassStaticInstance => UNITY_EXAMPLE,
        StdlibItemId::DurationFromFrames
        | StdlibItemId::DurationFromMilliseconds
        | StdlibItemId::DurationFromParts
        | StdlibItemId::DurationFromSeconds
        | StdlibItemId::DurationFromWholeMilliseconds
        | StdlibItemId::DurationWholeSeconds
        | StdlibItemId::DurationSubsecondNanoseconds
        | StdlibItemId::DurationTotalSeconds
        | StdlibItemId::DurationTotalMilliseconds
        | StdlibItemId::DurationAdd
        | StdlibItemId::DurationSubtract => DURATION_EXAMPLE,
        StdlibItemId::GbaEmulatorRead => GBA_EXAMPLE,
        StdlibItemId::Print
        | StdlibItemId::SetVariable
        | StdlibItemId::SetTickRate
        | StdlibItemId::TimerState
        | StdlibItemId::TimerIsRunning
        | StdlibItemId::StringByteLength
        | StdlibItemId::StringContains
        | StdlibItemId::StringStartsWith
        | StdlibItemId::StringEndsWith
        | StdlibItemId::StringEqualsIgnoreAsciiCase
        | StdlibItemId::StringToAsciiLowerCase
        | StdlibItemId::StringReplaceAll
        | StdlibItemId::StringSplit
        | StdlibItemId::StringParse
        | StdlibItemId::StringSlice
        | StdlibItemId::StringConcat => BASIC_EXAMPLE,
        _ => BASIC_EXAMPLE,
    }
}

include!(concat!(env!("OUT_DIR"), "/stdlib_catalog.rs"));

#[cfg(test)]
mod tests {
    use crate::stdlib::{StandardLibrary, StdlibSymbolId};

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
        assert!(
            library
                .children_of(StdlibOwner::Type(StdlibTypeId::Process))
                .any(|child| child == StdlibSymbolId::Item(StdlibItemId::ProcessRead))
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
    fn source_generated_ids_schema_and_normalized_data_have_one_way_dependencies() {
        let source = include_str!("../../stdlib/standard.split");
        let ids = include_str!("ids.rs");
        let build = include_str!("../../build.rs");
        let generator = include_str!("../../crates/splitscript-stdlib-loader/src/generate.rs");
        let schema = include_str!("schema.rs");
        let declarations = include_str!("declarations.rs");
        let catalog = include_str!("catalog.rs");

        assert!(source.contains("stateProvider GBA as gba"));
        assert!(source.contains("intrinsic type String"));
        assert!(ids.contains("/stdlib_ids.rs"));
        assert!(!ids.contains("with_standard_library!("));
        assert!(build.contains("generate_ids"));
        assert!(build.contains("generate_catalog"));
        assert!(catalog.contains("/stdlib_catalog.rs"));
        assert!(generator.contains("pub fn generate_catalog"));
        let retired_macro = ["macro_rules! ", "standard_library"].concat();
        let retired_invocation = ["standard_", "library!"].concat();
        assert!(!catalog.contains(&retired_macro));
        assert!(!generator.contains(&retired_invocation));
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
    fn every_callable_is_authored_once_in_privileged_source() {
        let source = include_str!("../../stdlib/standard.split");

        for item in ITEMS {
            match item.implementation {
                Implementation::Intrinsic(intrinsic) => {
                    let annotation = format!("@intrinsic({})", intrinsic.name());
                    assert_eq!(
                        source.matches(&annotation).count(),
                        1,
                        "`{}` must have exactly one source declaration",
                        item.name
                    );
                }
                Implementation::LibraryBody { .. } => {
                    let declaration = format!("fn {}", item.name);
                    let position = source.find(&declaration).unwrap_or_else(|| {
                        panic!("`{}` must have a source body", item.qualified_name)
                    });
                    assert!(
                        matches!(
                            source.as_bytes().get(position + declaration.len()),
                            Some(b'(' | b'<')
                        ),
                        "`{}` must have a source body",
                        item.qualified_name
                    );
                }
            }
        }
    }

    #[test]
    fn declaration_type_expressions_use_catalog_constructor_identities() {
        let library = StandardLibrary::new();

        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Array)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Option)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Result)
                .parameters
                .iter()
                .map(|parameter| parameter.name)
                .collect::<Vec<_>>(),
            ["T"]
        );
        assert_eq!(
            library
                .type_constructor(StdlibTypeConstructorId::Set)
                .parameters[0]
                .constraints,
            [StdlibCapabilityId::Equatable]
        );
        assert_eq!(
            library.render_signature(StdlibItemId::ProcessRead),
            "Process.read<T>(address: address) -> T! where T: MemoryReadable"
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
        assert_eq!(
            library
                .capability(StdlibCapabilityId::Numeric)
                .super_capabilities,
            [StdlibCapabilityId::Equatable]
        );
        assert_eq!(
            library
                .capability(StdlibCapabilityId::Integer)
                .super_capabilities,
            [StdlibCapabilityId::Numeric, StdlibCapabilityId::Display]
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Numeric)
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Display)
        );
        assert!(
            library.capability_implies(StdlibCapabilityId::Integer, StdlibCapabilityId::Equatable)
        );
        assert!(library.capability_implies(StdlibCapabilityId::Float, StdlibCapabilityId::Numeric));
        assert!(
            !library.capability_implies(StdlibCapabilityId::Float, StdlibCapabilityId::Display)
        );
        assert_eq!(
            library.minimal_capabilities(&[
                StdlibCapabilityId::Integer,
                StdlibCapabilityId::Numeric,
                StdlibCapabilityId::Equatable,
                StdlibCapabilityId::Display,
            ]),
            [StdlibCapabilityId::Integer]
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Numeric)
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Display)
        );
        assert!(
            !library
                .core_type(CoreTypeId::U32)
                .capabilities
                .contains(&StdlibCapabilityId::Equatable)
        );
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Numeric));
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Display));
        assert!(library.core_type_has_capability(CoreTypeId::U32, StdlibCapabilityId::Equatable));
        assert!(library.core_type_has_capability(CoreTypeId::Bool, StdlibCapabilityId::Display));

        let schema = include_str!("schema.rs");
        let retired_constraint = ["enum Type", "Constraint"].concat();
        assert!(!schema.contains(&retired_constraint));
    }
}
