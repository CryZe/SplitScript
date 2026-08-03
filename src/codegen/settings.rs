//! Settings registration, refresh, decoding, and start-time initialization.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{EnumDecl, Program, SettingFileFilter, SettingKind, ValueId},
    semantic::SemanticModel,
    stdlib::{StandardLibrary, StdlibTypeId},
    types::TypeKind,
    wasm_ir,
};

use super::memory_plan::RuntimeScratch;
use super::{
    GcLayout, SettingStorage, Type, data_plan::StringPool, emit_default, emit_string_literal,
    enum_variant_payload, global_plan::RuntimeGlobals, imports::Abi, memarg,
};

/// Settings-only view of the completed backend plans.
pub(super) struct SettingsContext<'a> {
    pub standard_library: &'a StandardLibrary,
    pub abi: &'a Abi,
    pub enums: &'a [EnumDecl],
    pub gc: &'a GcLayout,
    pub globals: &'a HashMap<ValueId, u32>,
    pub runtime_globals: RuntimeGlobals,
    pub semantics: &'a SemanticModel,
    pub wasm_ir: &'a wasm_ir::Program,
}

pub(super) fn compile_string_from_memory(gc: &GcLayout) -> Function {
    let mut function = Function::new([
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
        (1, ValType::I32),
    ]);
    let pointer = 0;
    let length = 1;
    let output = 2;
    let index = 3;
    function
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(pointer))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Builds the allocation-free implementation behind
/// `SettingsView.enabled(key)`. The view parameter is `0` for current values
/// and `1` for previous values; the key is compared directly against UTF-8
/// bytes in the GC string rather than materializing one string per declaration.
pub(super) fn compile_settings_enabled(
    program: &Program,
    settings: &HashMap<ValueId, SettingStorage>,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let view = 0;
    let key = 1;
    let string_type = gc.standard_index(StdlibTypeId::String);

    for setting in &program.settings {
        if !matches!(setting.kind, SettingKind::Bool { .. }) {
            continue;
        }
        let storage = settings
            .get(&setting.id)
            .expect("boolean settings have current and previous storage");
        let bytes = setting.runtime_key().as_bytes();

        function
            .instruction(&Instruction::LocalGet(key))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::ArrayLen)
            .instruction(&Instruction::I32Const(bytes.len() as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(1));
        for (index, byte) in bytes.iter().copied().enumerate() {
            function
                .instruction(&Instruction::LocalGet(key))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::I32Const(index as i32))
                .instruction(&Instruction::ArrayGetU(string_type))
                .instruction(&Instruction::I32Const(i32::from(byte)))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::I32And);
        }
        function
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::LocalGet(view))
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
            .instruction(&Instruction::GlobalGet(storage.old))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::GlobalGet(storage.current))
            .instruction(&Instruction::End)
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
    }

    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_refresh_settings(
    program: &Program,
    lowering: &SettingsContext<'_>,
    strings: &StringPool,
    settings: &HashMap<ValueId, SettingStorage>,
    string_from_memory: u32,
    string_eq: u32,
    scratch: RuntimeScratch,
) -> Function {
    let settings_length = scratch.settings_length.start();
    let semantics = lowering.semantics;
    let abi = lowering.abi;
    let enums = lowering.enums;
    let mut function = Function::new([
        (2, ValType::I64),
        (
            1,
            lowering.gc.val_type(Type::Standard(StdlibTypeId::String)),
        ),
    ]);
    let map = 0;
    let value = 1;
    let decoded = 2;
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::SettingsMapLoad),
        ))
        .instruction(&Instruction::LocalSet(map));

    for setting in &program.settings {
        let Some(storage) = settings.get(&setting.id).copied() else {
            continue;
        };
        function
            .instruction(&Instruction::GlobalGet(storage.current))
            .instruction(&Instruction::GlobalSet(storage.old));
        emit_setting_default(
            &mut function,
            setting,
            storage.current,
            enums,
            semantics,
            lowering.gc,
        );

        let (key_ptr, key_len) = strings.get(setting.runtime_key());
        function
            .instruction(&Instruction::LocalGet(map))
            .instruction(&Instruction::I32Const(key_ptr as i32))
            .instruction(&Instruction::I32Const(key_len as i32))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::SettingsMapGet),
            ))
            .instruction(&Instruction::LocalTee(value))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else);
        match &setting.kind {
            SettingKind::Bool { .. } => {
                function
                    .instruction(&Instruction::LocalGet(value))
                    .instruction(&Instruction::I32Const(settings_length))
                    .instruction(&Instruction::Call(
                        abi.function(AbiImportId::SettingValueGetBool),
                    ))
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(settings_length))
                    .instruction(&Instruction::I32Load8U(memarg()))
                    .instruction(&Instruction::GlobalSet(storage.current))
                    .instruction(&Instruction::End);
            }
            SettingKind::Choice { options, .. } => {
                let setting_type = semantics
                    .value_type(setting.id)
                    .expect("checked choice settings have value types");
                let TypeKind::Enum(enumeration) = semantics.types().kind(setting_type) else {
                    unreachable!("checked choice settings use source enums")
                };
                emit_load_setting_string(
                    &mut function,
                    abi,
                    value,
                    decoded,
                    string_from_memory,
                    scratch,
                );
                function.instruction(&Instruction::If(BlockType::Empty));
                for option in options {
                    function.instruction(&Instruction::LocalGet(decoded));
                    emit_string_literal(&mut function, &option.variant, lowering.gc);
                    function
                        .instruction(&Instruction::Call(string_eq))
                        .instruction(&Instruction::If(BlockType::Empty));
                    emit_enum_variant(
                        &mut function,
                        *enumeration,
                        semantics
                            .setting_choice_option(option.id)
                            .expect("checked choice options have resolved variants"),
                        enums,
                        semantics,
                        lowering.gc,
                    );
                    function
                        .instruction(&Instruction::GlobalSet(storage.current))
                        .instruction(&Instruction::End);
                }
                function.instruction(&Instruction::End);
            }
            SettingKind::File { .. } => {
                emit_load_setting_string(
                    &mut function,
                    abi,
                    value,
                    decoded,
                    string_from_memory,
                    scratch,
                );
                function
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::LocalGet(decoded))
                    .instruction(&Instruction::GlobalSet(storage.current))
                    .instruction(&Instruction::End);
            }
            SettingKind::Title { .. } => unreachable!(),
        }
        function
            .instruction(&Instruction::LocalGet(value))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::SettingValueFree),
            ))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::LocalGet(map))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::SettingsMapFree),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_load_setting_string(
    function: &mut Function,
    abi: &Abi,
    value: u32,
    decoded: u32,
    string_from_memory: u32,
    scratch: RuntimeScratch,
) {
    let settings_length = scratch.settings_length.start();
    let settings_string = scratch.settings_string.start();
    let settings_string_capacity = scratch.settings_string.capacity();
    function
        .instruction(&Instruction::I32Const(settings_length))
        .instruction(&Instruction::I32Const(settings_string_capacity))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::I32Const(settings_string))
        .instruction(&Instruction::I32Const(settings_length))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::SettingValueGetString),
        ))
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(settings_string))
        .instruction(&Instruction::I32Const(settings_length))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::LocalSet(decoded))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
}

fn emit_setting_default(
    function: &mut Function,
    setting: &crate::ast::SettingDecl,
    global: u32,
    enums: &[EnumDecl],
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    match &setting.kind {
        SettingKind::Bool { default } => {
            function.instruction(&Instruction::I32Const(*default as i32));
        }
        SettingKind::Choice { .. } => {
            let setting_type = semantics
                .value_type(setting.id)
                .expect("checked choice settings have value types");
            let TypeKind::Enum(enumeration) = semantics.types().kind(setting_type) else {
                unreachable!("checked choice settings use source enums")
            };
            emit_enum_variant(
                function,
                *enumeration,
                semantics
                    .setting_choice_default(setting.id)
                    .expect("checked choice settings have resolved defaults"),
                enums,
                semantics,
                gc,
            );
        }
        SettingKind::File { .. } => emit_string_literal(function, "", gc),
        SettingKind::Title { .. } => return,
    }
    function.instruction(&Instruction::GlobalSet(global));
}

pub(super) fn emit_setting_registration(
    function: &mut Function,
    setting: &crate::ast::SettingDecl,
    strings: &StringPool,
    storage: Option<SettingStorage>,
    lowering: &SettingsContext<'_>,
) {
    let abi = lowering.abi;
    let enums = lowering.enums;
    let semantics = lowering.semantics;
    let gc = lowering.gc;
    let (key_ptr, key_len) = strings.get(setting.runtime_key());
    let (description_ptr, description_len) = strings.get(&setting.description);
    match &setting.kind {
        SettingKind::Bool { default } => {
            let storage = storage.unwrap();
            function
                .instruction(&Instruction::I32Const(key_ptr as i32))
                .instruction(&Instruction::I32Const(key_len as i32))
                .instruction(&Instruction::I32Const(description_ptr as i32))
                .instruction(&Instruction::I32Const(description_len as i32))
                .instruction(&Instruction::I32Const(*default as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::UserSettingsAddBool),
                ))
                .instruction(&Instruction::GlobalSet(storage.current))
                .instruction(&Instruction::GlobalGet(storage.current))
                .instruction(&Instruction::GlobalSet(storage.old));
        }
        SettingKind::Title { heading_level } => {
            function
                .instruction(&Instruction::I32Const(key_ptr as i32))
                .instruction(&Instruction::I32Const(key_len as i32))
                .instruction(&Instruction::I32Const(description_ptr as i32))
                .instruction(&Instruction::I32Const(description_len as i32))
                .instruction(&Instruction::I32Const(*heading_level as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::UserSettingsAddTitle),
                ));
        }
        SettingKind::Choice {
            default_variant,
            options,
            ..
        } => {
            let storage = storage.unwrap();
            let setting_type = semantics
                .value_type(setting.id)
                .expect("checked choice settings have value types");
            let TypeKind::Enum(enumeration) = semantics.types().kind(setting_type) else {
                unreachable!("checked choice settings use source enums")
            };
            emit_enum_variant(
                function,
                *enumeration,
                semantics
                    .setting_choice_default(setting.id)
                    .expect("checked choice settings have resolved defaults"),
                enums,
                semantics,
                gc,
            );
            function
                .instruction(&Instruction::GlobalSet(storage.current))
                .instruction(&Instruction::GlobalGet(storage.current))
                .instruction(&Instruction::GlobalSet(storage.old));
            let (default_ptr, default_len) = strings.get(default_variant);
            function
                .instruction(&Instruction::I32Const(key_ptr as i32))
                .instruction(&Instruction::I32Const(key_len as i32))
                .instruction(&Instruction::I32Const(description_ptr as i32))
                .instruction(&Instruction::I32Const(description_len as i32))
                .instruction(&Instruction::I32Const(default_ptr as i32))
                .instruction(&Instruction::I32Const(default_len as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::UserSettingsAddChoice),
                ));
            for option in options {
                let (option_ptr, option_len) = strings.get(&option.variant);
                let (option_description_ptr, option_description_len) =
                    strings.get(&option.description);
                function
                    .instruction(&Instruction::I32Const(key_ptr as i32))
                    .instruction(&Instruction::I32Const(key_len as i32))
                    .instruction(&Instruction::I32Const(option_ptr as i32))
                    .instruction(&Instruction::I32Const(option_len as i32))
                    .instruction(&Instruction::I32Const(option_description_ptr as i32))
                    .instruction(&Instruction::I32Const(option_description_len as i32))
                    .instruction(&Instruction::Call(
                        abi.function(AbiImportId::UserSettingsAddChoiceOption),
                    ))
                    .instruction(&Instruction::Drop);
            }
        }
        SettingKind::File { filters } => {
            let storage = storage.unwrap();
            emit_string_literal(function, "", gc);
            function
                .instruction(&Instruction::GlobalSet(storage.current))
                .instruction(&Instruction::GlobalGet(storage.current))
                .instruction(&Instruction::GlobalSet(storage.old))
                .instruction(&Instruction::I32Const(key_ptr as i32))
                .instruction(&Instruction::I32Const(key_len as i32))
                .instruction(&Instruction::I32Const(description_ptr as i32))
                .instruction(&Instruction::I32Const(description_len as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::UserSettingsAddFileSelect),
                ));
            for filter in filters {
                match filter {
                    SettingFileFilter::Name {
                        description,
                        pattern,
                    } => {
                        let (description_ptr, description_len) = description
                            .as_ref()
                            .map_or((0, 0), |description| strings.get(description));
                        let (pattern_ptr, pattern_len) = strings.get(pattern);
                        function
                            .instruction(&Instruction::I32Const(key_ptr as i32))
                            .instruction(&Instruction::I32Const(key_len as i32))
                            .instruction(&Instruction::I32Const(description_ptr as i32))
                            .instruction(&Instruction::I32Const(description_len as i32))
                            .instruction(&Instruction::I32Const(pattern_ptr as i32))
                            .instruction(&Instruction::I32Const(pattern_len as i32))
                            .instruction(&Instruction::Call(
                                abi.function(AbiImportId::UserSettingsAddFileSelectNameFilter),
                            ));
                    }
                    SettingFileFilter::Mime(mime) => {
                        let (mime_ptr, mime_len) = strings.get(mime);
                        function
                            .instruction(&Instruction::I32Const(key_ptr as i32))
                            .instruction(&Instruction::I32Const(key_len as i32))
                            .instruction(&Instruction::I32Const(mime_ptr as i32))
                            .instruction(&Instruction::I32Const(mime_len as i32))
                            .instruction(&Instruction::Call(
                                abi.function(AbiImportId::UserSettingsAddFileSelectMimeFilter),
                            ));
                    }
                }
            }
        }
    }
    if let Some(tooltip) = &setting.tooltip {
        let (tooltip_ptr, tooltip_len) = strings.get(tooltip);
        function
            .instruction(&Instruction::I32Const(key_ptr as i32))
            .instruction(&Instruction::I32Const(key_len as i32))
            .instruction(&Instruction::I32Const(tooltip_ptr as i32))
            .instruction(&Instruction::I32Const(tooltip_len as i32))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::UserSettingsSetTooltip),
            ));
    }
}

pub(super) fn emit_enum_variant(
    function: &mut Function,
    enumeration: crate::ast::EnumId,
    variant: crate::ast::EnumVariantId,
    enums: &[EnumDecl],
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    let declaration = enums.iter().find(|item| item.id == enumeration).unwrap();
    let selected = declaration
        .variants
        .iter()
        .position(|item| item.id == variant)
        .unwrap();
    function.instruction(&Instruction::I32Const(selected as i32));
    for declared in &declaration.variants {
        if let Some(payload_type) = enum_variant_payload(declared.id, semantics) {
            emit_default(function, payload_type, gc);
        } else {
            function.instruction(&Instruction::I32Const(0));
        }
    }
    function.instruction(&Instruction::StructNew(gc.index(Type::Enum(enumeration))));
}
