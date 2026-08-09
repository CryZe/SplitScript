//! Descriptor-driven orchestration for compiler-generated runtime helpers.

use std::collections::HashMap;

use wasm_encoder::Function;

use crate::{
    ast::{Program, ValueId},
    intrinsic_registry::RuntimeHelperId,
    stdlib::StdlibTypeId,
    types::ResolvedArrayType,
};

use super::data_plan::StringPool;
use super::imports::Abi;
use super::memory_plan::LinearMemoryLayout;
use super::runtime_helper_registry;
use super::{GcLayout, RuntimeHelperPlan, SettingStorage, Type, settings, try_array_element_type};

mod equality;
mod gba;
mod process;
mod strings;
mod unity;

pub(super) use equality::{compile_equality, emit_value_equality};

pub(super) struct RuntimeHelperInputs<'a> {
    pub abi: &'a Abi,
    pub strings: &'a StringPool,
    pub plan: &'a RuntimeHelperPlan,
    pub arrays: &'a [ResolvedArrayType],
    pub program: &'a Program,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub settings: &'a settings::SettingsContext<'a>,
    pub settings_map: &'a HashMap<ValueId, SettingStorage>,
    pub gc: &'a GcLayout,
    pub memory: LinearMemoryLayout,
}

pub(super) fn compile_runtime(
    plan: &RuntimeHelperPlan,
    inputs: &RuntimeHelperInputs<'_>,
) -> Vec<Function> {
    plan.entries()
        .map(|helper| (runtime_helper_registry::descriptor(helper).build_body)(inputs))
        .collect()
}

fn array_layout(inputs: &RuntimeHelperInputs<'_>, element: Type) -> u32 {
    inputs.gc.index(Type::Array(
        inputs
            .arrays
            .iter()
            .find(|array| try_array_element_type(array.id, inputs.semantics) == Some(element))
            .expect("runtime helper has its required reachable array layout")
            .id,
    ))
}

pub(super) fn build_print_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_print_string(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_timer_set_variable(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_timer_set_variable(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_format_i64(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_format_i64(inputs.gc)
}

pub(super) fn build_string_equality(inputs: &RuntimeHelperInputs<'_>) -> Function {
    equality::compile_string_eq(inputs.gc)
}

pub(super) fn build_string_match(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_match(inputs.gc)
}

pub(super) fn build_string_find(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_find(inputs.gc)
}

pub(super) fn build_string_to_ascii_lower_case(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_to_ascii_lower_case(inputs.gc)
}

pub(super) fn build_string_replace_all(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_replace_all(
        inputs.plan.function(RuntimeHelperId::StringFind),
        inputs.gc,
    )
}

pub(super) fn build_string_slice(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_slice(inputs.gc)
}

pub(super) fn build_scan_process_range(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_scan_process_range(inputs.abi, inputs.memory.scratch().scan)
}

pub(super) fn build_read_relative32(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_relative32(inputs.abi, inputs.memory.scratch().abi_read)
}

pub(super) fn build_read_utf8_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_utf8_string(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch().native_utf8,
    )
}

pub(super) fn build_utf16_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    process::compile_utf16_string_from_memory(inputs.gc, scratch.utf16_input, scratch.utf16_output)
}

pub(super) fn build_read_utf16_le_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_utf16_le_string(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::Utf16StringFromMemory),
        inputs.gc,
        inputs.memory.scratch().utf16_input,
    )
}

pub(super) fn build_read_managed_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    process::compile_read_managed_string(
        inputs.abi,
        inputs.gc,
        scratch.abi_read,
        scratch.utf16_input,
        scratch.utf16_output,
    )
}

pub(super) fn build_module_path(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_module_path(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_process_path(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_process_path(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_runtime_operating_system(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_runtime_metadata(
        inputs.abi,
        crate::abi::AbiImportId::RuntimeGetOs,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_runtime_architecture(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_runtime_metadata(
        inputs.abi,
        crate::abi::AbiImportId::RuntimeGetArch,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_c_string_equality(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_c_string_eq(inputs.abi, inputs.gc, inputs.memory.scratch().c_string)
}

pub(super) fn build_backing_field_equality(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_backing_field_eq(inputs.abi, inputs.gc, inputs.memory.scratch().c_string)
}

pub(super) fn build_unity_get_image(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_unity_get_image(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::CStringEquality),
        inputs.gc,
        inputs.memory.scratch().abi_read,
    )
}

pub(super) fn build_unity_get_class(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_unity_get_class(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::CStringEquality),
        inputs.gc,
        inputs.memory.scratch().abi_read,
    )
}

pub(super) fn build_unity_get_field_offset(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_unity_get_field_offset(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::CStringEquality),
        inputs.plan.function(RuntimeHelperId::BackingFieldEquality),
        inputs.gc,
        inputs.memory.scratch().abi_read,
    )
}

pub(super) fn build_unity_get_field_any(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_unity_get_field_any(
        inputs.plan.function(RuntimeHelperId::UnityGetFieldOffset),
        array_layout(inputs, Type::Standard(StdlibTypeId::String)),
        inputs.gc,
    )
}

pub(super) fn build_unity_get_static_instance(inputs: &RuntimeHelperInputs<'_>) -> Function {
    unity::compile_unity_get_static_instance(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::UnityGetFieldAny),
        inputs.gc,
        inputs.memory.scratch().abi_read,
    )
}

pub(super) fn build_concat_strings(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_concat_strings(
        array_layout(inputs, Type::Standard(StdlibTypeId::String)),
        inputs.gc,
    )
}

pub(super) fn build_follow_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_follow_address(
        inputs.abi,
        array_layout(inputs, Type::I64),
        inputs.memory.scratch().abi_read,
    )
}

pub(super) fn build_gba_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    gba::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_string_from_memory(inputs.gc)
}

pub(super) fn build_refresh_settings(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_refresh_settings(
        inputs.program,
        inputs.settings,
        inputs.strings,
        inputs.settings_map,
        inputs
            .plan
            .optional_function(RuntimeHelperId::StringFromMemory)
            .unwrap_or(0),
        inputs
            .plan
            .optional_function(RuntimeHelperId::StringEquality)
            .unwrap_or(0),
        inputs.memory.scratch(),
    )
}

pub(super) fn build_settings_enabled(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_settings_enabled(inputs.program, inputs.settings_map, inputs.gc)
}
