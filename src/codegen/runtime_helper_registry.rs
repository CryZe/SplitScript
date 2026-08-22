//! One trusted description of every compiler-generated runtime helper.
//!
//! Dependency closure, function signatures, deterministic ordering, and body
//! construction all consume these descriptors. A helper therefore cannot be
//! added to one backend phase while being silently omitted from another.

use wasm_encoder::{Function, ValType};

use std::collections::{BTreeSet, HashMap};

use crate::{
    abi::{AbiCatalog, AbiEffect, AbiImportId},
    ast::ArrayTypeId,
    intrinsic_registry::{self, DependencyRoot, RuntimeHelperId},
    stdlib::{Effect, EffectSet, IntrinsicId, StdlibTypeId},
    types::ResolvedArrayType,
};

use super::{GcLayout, Type, runtime_helpers, try_array_element_type};

pub(super) struct RuntimeHelperPlan {
    pub ordered: Vec<RuntimeHelperId>,
    pub functions: HashMap<RuntimeHelperId, u32>,
}

impl RuntimeHelperPlan {
    pub(super) fn function(&self, helper: RuntimeHelperId) -> u32 {
        self.functions[&helper]
    }

    pub(super) fn optional_function(&self, helper: RuntimeHelperId) -> Option<u32> {
        self.functions.get(&helper).copied()
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = RuntimeHelperId> + '_ {
        self.ordered.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HelperValueType {
    I32,
    I64,
    F64,
    String,
    Standard(StdlibTypeId),
    StringArray,
    I64Array,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HelperSignature {
    pub params: &'static [HelperValueType],
    pub results: &'static [HelperValueType],
}

pub(super) type BodyBuilder = for<'a> fn(&runtime_helpers::RuntimeHelperInputs<'a>) -> Function;

#[derive(Clone, Copy)]
pub(super) struct RuntimeHelperDescriptor {
    pub id: RuntimeHelperId,
    pub signature: HelperSignature,
    pub dependencies: &'static [RuntimeHelperId],
    pub host_imports: &'static [AbiImportId],
    pub build_body: BodyBuilder,
}

macro_rules! helper {
    ($id:ident, ($($param:expr),* $(,)?) -> ($($result:expr),* $(,)?),
     deps [$($dependency:ident),* $(,)?], imports [$($import:ident),* $(,)?], $builder:ident) => {
        RuntimeHelperDescriptor {
            id: RuntimeHelperId::$id,
            signature: HelperSignature {
                params: &[$($param),*],
                results: &[$($result),*],
            },
            dependencies: &[$(RuntimeHelperId::$dependency),*],
            host_imports: &[$(AbiImportId::$import),*],
            build_body: runtime_helpers::$builder,
        }
    };
}

use HelperValueType::{F64, I32, I64, I64Array, Standard, String as StringValue, StringArray};

/// Canonical function-index and body order. Dependencies always occur before
/// the helpers that call them, which is validated by the registry tests.
pub(super) const DESCRIPTORS: &[RuntimeHelperDescriptor] = &[
    helper!(PrintString, (StringValue) -> (), deps [], imports [RuntimePrintMessage], build_print_string),
    helper!(TimerSetVariable, (StringValue, StringValue) -> (), deps [], imports [TimerSetVariable], build_timer_set_variable),
    helper!(FormatI64, (I64, I32, I32) -> (StringValue), deps [], imports [], build_format_i64),
    helper!(FormatChar, (I32) -> (StringValue), deps [], imports [], build_format_char),
    helper!(StringEquality, (StringValue, StringValue) -> (I32), deps [], imports [], build_string_equality),
    helper!(StringMatch, (StringValue, StringValue, I32) -> (I32), deps [], imports [], build_string_match),
    helper!(StringFind, (StringValue, StringValue, I32) -> (I32), deps [], imports [], build_string_find),
    helper!(StringRFind, (StringValue, StringValue) -> (I32), deps [], imports [], build_string_rfind),
    helper!(StringAsciiCase, (StringValue, I32) -> (StringValue), deps [], imports [], build_string_ascii_case),
    helper!(StringReplaceAll, (StringValue, StringValue, StringValue) -> (StringValue), deps [StringFind], imports [], build_string_replace_all),
    helper!(StringSplit, (StringValue, StringValue) -> (StringArray), deps [StringFind], imports [], build_string_split),
    helper!(StringParseInteger, (StringValue, I32, I64, I64) -> (I32, I64), deps [], imports [], build_string_parse_integer),
    helper!(DecimalLeftShift, (I32, I32, I32, I32) -> (I32, I32, I32), deps [], imports [], build_decimal_left_shift),
    helper!(DecimalRightShift, (I32, I32, I32, I32) -> (I32, I32, I32), deps [], imports [], build_decimal_right_shift),
    helper!(DecimalRound, (I32, I32, I32) -> (I64), deps [], imports [], build_decimal_round),
    helper!(StringParseFloat, (StringValue, I32) -> (I32, F64), deps [DecimalLeftShift, DecimalRightShift, DecimalRound], imports [], build_string_parse_float),
    helper!(StringInspect, (StringValue, I32, I32) -> (I32, I32), deps [], imports [], build_string_inspect),
    helper!(StringSlice, (StringValue, I32, I32) -> (StringValue), deps [], imports [], build_string_slice),
    helper!(StringTrimAsciiWhitespace, (StringValue) -> (StringValue), deps [StringSlice], imports [], build_string_trim_ascii_whitespace),
    helper!(StringPad, (StringValue, I32, I32, I32) -> (StringValue), deps [], imports [], build_string_pad),
    helper!(ScanProcessRange, (I64, I64, I64, I32, I32, I32) -> (I64), deps [], imports [ProcessRead], build_scan_process_range),
    helper!(ReadRelative32, (I64, I64) -> (I64), deps [], imports [ProcessRead], build_read_relative32),
    helper!(ScanRelative32TargetRange, (I64, I64, I64, I32, I32, I32, I64, I64) -> (I64), deps [ScanProcessRange, ReadRelative32], imports [], build_scan_relative32_target_range),
    helper!(StringFromMemory, (I32, I32) -> (StringValue), deps [], imports [], build_string_from_memory),
    helper!(Utf16StringFromMemory, (I32) -> (StringValue), deps [], imports [], build_utf16_string_from_memory),
    helper!(ReadUtf8String, (I64, I64, I32) -> (StringValue), deps [StringFromMemory], imports [ProcessRead], build_read_utf8_string),
    helper!(ReadUtf16LeString, (I64, I64, I32) -> (StringValue), deps [Utf16StringFromMemory], imports [ProcessRead], build_read_utf16_le_string),
    helper!(ReadManagedString, (I64, I64, I32) -> (StringValue), deps [], imports [ProcessRead], build_read_managed_string),
    helper!(LoadedModule, (I64, StringValue) -> (Standard(StdlibTypeId::Module)), deps [], imports [ProcessGetModuleAddress, ProcessGetModuleSize], build_loaded_module),
    helper!(ModulePath, (I64, StringValue) -> (StringValue), deps [StringFromMemory], imports [ProcessGetModulePath], build_module_path),
    helper!(ProcessPath, (I64) -> (StringValue), deps [StringFromMemory], imports [ProcessGetPath], build_process_path),
    helper!(RuntimeOperatingSystem, () -> (StringValue), deps [StringFromMemory], imports [RuntimeGetOs], build_runtime_operating_system),
    helper!(RuntimeArchitecture, () -> (StringValue), deps [StringFromMemory], imports [RuntimeGetArch], build_runtime_architecture),
    helper!(CStringEquality, (I64, I64, StringValue, I32, I32) -> (I32), deps [], imports [ProcessRead], build_c_string_equality),
    helper!(BackingFieldEquality, (I64, I64, StringValue) -> (I32), deps [], imports [ProcessRead], build_backing_field_equality),
    helper!(UnityGetImage, (I64, Standard(StdlibTypeId::UnityModule), StringValue) -> (Standard(StdlibTypeId::UnityImage)), deps [CStringEquality], imports [ProcessRead], build_unity_get_image),
    helper!(UnityGetClass, (I64, Standard(StdlibTypeId::UnityImage), StringValue) -> (Standard(StdlibTypeId::UnityClass)), deps [CStringEquality], imports [ProcessRead], build_unity_get_class),
    helper!(UnityGetFieldOffset, (I64, Standard(StdlibTypeId::UnityClass), StringValue) -> (I64), deps [CStringEquality, BackingFieldEquality], imports [ProcessRead], build_unity_get_field_offset),
    helper!(UnityGetFieldAny, (I64, Standard(StdlibTypeId::UnityClass), StringArray) -> (Standard(StdlibTypeId::UnityField)), deps [UnityGetFieldOffset], imports [], build_unity_get_field_any),
    helper!(UnityGetStaticInstance, (I64, Standard(StdlibTypeId::UnityClass), StringArray) -> (I64), deps [UnityGetFieldAny], imports [ProcessRead], build_unity_get_static_instance),
    helper!(JoinStrings, (StringArray, StringValue) -> (StringValue), deps [], imports [], build_join_strings),
    helper!(FollowAddress, (I64, I64, I64Array) -> (I64), deps [], imports [ProcessRead], build_follow_address),
    helper!(GBATranslateAddress, (I64, Standard(StdlibTypeId::GBAEmulator), I32, I32) -> (I64), deps [], imports [ProcessRead], build_gba_translate_address),
    helper!(Ps2TranslateAddress, (I64, Standard(StdlibTypeId::PS2Emulator), I32, I32) -> (I64), deps [], imports [ProcessRead], build_ps2_translate_address),
    helper!(Ps1TranslateAddress, (I64, Standard(StdlibTypeId::PS1Emulator), I32, I32) -> (I64), deps [], imports [ProcessRead], build_ps1_translate_address),
    helper!(RefreshSettings, () -> (), deps [], imports [], build_refresh_settings),
    helper!(SettingsEnabled, (I32, StringValue) -> (I32), deps [], imports [], build_settings_enabled),
    helper!(SettingsContains, (I32, StringValue) -> (I32), deps [], imports [], build_settings_contains),
];

const _: [(); RuntimeHelperId::COUNT] = [(); DESCRIPTORS.len()];

pub(super) fn descriptor(id: RuntimeHelperId) -> &'static RuntimeHelperDescriptor {
    let descriptor = &DESCRIPTORS[id.index()];
    debug_assert_eq!(descriptor.id, id);
    descriptor
}

/// Cross-checks observable public effects against every transitive generated
/// helper and host import used by the intrinsic implementation.
pub(super) fn validate_intrinsic_effects() -> Vec<String> {
    let mut errors = Vec::new();
    for intrinsic in IntrinsicId::ALL {
        let contract = intrinsic_registry::contract(*intrinsic);
        let mut implementation = EffectSet::none();
        let mut visited = BTreeSet::new();
        for root in contract.dependency_roots {
            collect_root_effects(*root, &mut implementation, &mut visited, &mut errors);
        }
        for effect in [
            Effect::ReadsTimer,
            Effect::ReadsRuntime,
            Effect::ReadsProcess,
            Effect::WritesCurrentState,
            Effect::WritesTimer,
            Effect::WritesRuntime,
        ] {
            if contract.effects.contains(&effect) != implementation.contains(&effect) {
                errors.push(format!(
                    "intrinsic `{intrinsic:?}` declares `{}` inconsistently with its helper/ABI roots",
                    effect.name()
                ));
            }
        }
    }
    errors
}

fn collect_root_effects(
    root: DependencyRoot,
    effects: &mut EffectSet,
    visited: &mut BTreeSet<RuntimeHelperId>,
    errors: &mut Vec<String>,
) {
    match root {
        DependencyRoot::Helper(helper) => {
            if !visited.insert(helper) {
                return;
            }
            let descriptor = descriptor(helper);
            for dependency in descriptor.dependencies {
                collect_root_effects(
                    DependencyRoot::Helper(*dependency),
                    effects,
                    visited,
                    errors,
                );
            }
            for import in descriptor.host_imports {
                collect_abi_effects(*import, effects, errors);
            }
        }
        DependencyRoot::HostImport(import) => collect_abi_effects(import, effects, errors),
    }
}

fn collect_abi_effects(import: AbiImportId, effects: &mut EffectSet, errors: &mut Vec<String>) {
    for effect in AbiCatalog::new().import(import).effects {
        let mapped = match effect {
            AbiEffect::ReadsTimer => Some(Effect::ReadsTimer),
            AbiEffect::ReadsRuntime => Some(Effect::ReadsRuntime),
            AbiEffect::WritesTimer => Some(Effect::WritesTimer),
            AbiEffect::WritesRuntime => Some(Effect::WritesRuntime),
            AbiEffect::ReadsProcess => Some(Effect::ReadsProcess),
            AbiEffect::ManagesProcess | AbiEffect::RegistersSettings | AbiEffect::ReadsSettings => {
                None
            }
        };
        if let Some(mapped) = mapped {
            *effects = effects.with(mapped);
        } else {
            errors.push(format!(
                "intrinsic dependency `{import:?}` exposes unsupported ABI effect `{effect:?}`"
            ));
        }
    }
}

pub(super) fn resolve_signature(
    signature: HelperSignature,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
) -> (Vec<ValType>, Vec<ValType>) {
    let resolve = |ty| match ty {
        HelperValueType::I32 => ValType::I32,
        HelperValueType::I64 => ValType::I64,
        HelperValueType::F64 => ValType::F64,
        HelperValueType::String => gc.val_type(Type::Standard(StdlibTypeId::String)),
        HelperValueType::Standard(standard) => gc.val_type(Type::Standard(standard)),
        HelperValueType::StringArray | HelperValueType::I64Array => gc.val_type(Type::Array(
            required_array_layout(ty, arrays, semantics)
                .expect("runtime array helper has a reachable layout"),
        )),
    };
    (
        signature.params.iter().copied().map(resolve).collect(),
        signature.results.iter().copied().map(resolve).collect(),
    )
}

pub(super) fn required_array_layouts(
    helpers: impl IntoIterator<Item = RuntimeHelperId>,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
) -> impl Iterator<Item = ArrayTypeId> {
    helpers
        .into_iter()
        .flat_map(|helper| {
            let signature = descriptor(helper).signature;
            signature.params.iter().chain(signature.results)
        })
        .filter_map(|ty| required_array_layout(*ty, arrays, semantics))
        .collect::<BTreeSet<_>>()
        .into_iter()
}

fn required_array_layout(
    ty: HelperValueType,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
) -> Option<ArrayTypeId> {
    let element = match ty {
        HelperValueType::StringArray => Type::Standard(StdlibTypeId::String),
        HelperValueType::I64Array => Type::I64,
        HelperValueType::I32
        | HelperValueType::I64
        | HelperValueType::F64
        | HelperValueType::String
        | HelperValueType::Standard(_) => return None,
    };
    arrays
        .iter()
        .find(|array| try_array_element_type(array.id, semantics) == Some(element))
        .map(|array| array.id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::intrinsic_registry::RuntimeHelperId;

    use super::{DESCRIPTORS, validate_intrinsic_effects};

    #[test]
    fn registry_is_complete_unique_and_dependency_ordered() {
        assert_eq!(DESCRIPTORS.len(), RuntimeHelperId::COUNT);
        let mut seen = BTreeSet::new();
        for (index, descriptor) in DESCRIPTORS.iter().enumerate() {
            assert_eq!(descriptor.id.index(), index);
            assert!(seen.insert(descriptor.id));
            assert_eq!(
                descriptor
                    .dependencies
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len(),
                descriptor.dependencies.len(),
                "{:?} repeats a helper dependency",
                descriptor.id
            );
            assert_eq!(
                descriptor
                    .host_imports
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len(),
                descriptor.host_imports.len(),
                "{:?} repeats an ABI import",
                descriptor.id
            );
            for dependency in descriptor.dependencies {
                assert!(
                    seen.contains(dependency),
                    "{:?} must be ordered after dependency {:?}",
                    descriptor.id,
                    dependency
                );
            }
        }
    }

    #[test]
    fn backend_phases_consume_the_descriptor_and_runtime_plan() {
        let dependency_planner = include_str!("dependencies.rs");
        let function_planner = include_str!("function_plan.rs");
        let body_builder = include_str!("runtime_helpers.rs");

        assert!(!dependency_planner.contains("match helper"));
        assert!(!function_planner.contains("match helper"));
        assert!(!body_builder.contains("match helper"));
        assert!(function_planner.contains("resolve_signature"));
        assert!(body_builder.contains("build_body"));
        assert!(body_builder.contains("plan.entries()"));
    }

    #[test]
    fn intrinsic_effects_match_transitive_helper_and_abi_roots() {
        assert_eq!(validate_intrinsic_effects(), Vec::<String>::new());
    }
}
