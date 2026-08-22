//! One-time module initialization after all backend plans are fixed.

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction};

use crate::{
    abi::AbiImportId,
    ast::{Program, RecordDecl, RecordId, ValueId},
    semantic::{ResolvedEnumVariantId, SemanticModel},
    stdlib::{StandardLibrary, StdlibTypeId},
    types::{TypeId, TypeKind},
    wasm_ir,
};

use super::{
    GcLayout, STATE_TYPE, SettingStorage, Type,
    context::EmissionContext,
    data_plan::StringPool,
    debug_artifacts::DebugEmission,
    emit_default,
    expression::{BareReturn, ExprContext, LocalStorage, MatchLayout, compile_expr},
    semantic_type,
    settings::{SettingsContext, emit_enum_variant, emit_setting_registration},
    value_type,
};

pub(super) struct StartFunctions {
    pub(super) start: u32,
    pub(super) refresh_settings: Option<u32>,
    pub(super) setup: Option<u32>,
}

pub(super) fn compile_start(
    program: &Program,
    settings_context: &SettingsContext<'_>,
    emission: &EmissionContext<'_>,
    strings: &StringPool,
    settings: &HashMap<ValueId, SettingStorage>,
    start_functions: StartFunctions,
    has_async_frame: bool,
) -> Function {
    let semantics = settings_context.semantics;
    let mut function = Function::new([]);
    let debug = emission.debug_emission(start_functions.start);
    emit_enum_global_initializers(&mut function, program, settings_context, debug);
    emit_aggregate_global_initializers(&mut function, program, emission, debug);
    emit_initial_state(&mut function, program, semantics, settings_context.gc);
    function.instruction(&Instruction::GlobalSet(
        settings_context.runtime_globals.current,
    ));
    emit_initial_state(&mut function, program, semantics, settings_context.gc);
    function.instruction(&Instruction::GlobalSet(
        settings_context.runtime_globals.old,
    ));
    if has_async_frame {
        function
            .instruction(&Instruction::StructNewDefault(
                settings_context.gc.async_frame_index(),
            ))
            .instruction(&Instruction::GlobalSet(
                settings_context.runtime_globals.async_frame,
            ));
    }
    for setting in &program.settings {
        emit_setting_registration(
            &mut function,
            setting,
            strings,
            settings.get(&setting.id).copied(),
            settings_context,
        );
    }
    if let Some(refresh_settings) = start_functions.refresh_settings {
        function.instruction(&Instruction::Call(refresh_settings));
    }
    function
        .instruction(&Instruction::F64Const(program.detached_tick_rate().into()))
        .instruction(&Instruction::Call(
            emission.abi.function(AbiImportId::RuntimeSetTickRate),
        ));
    if let Some(setup) = start_functions.setup {
        function.instruction(&Instruction::Call(setup));
    }
    function.instruction(&Instruction::End);
    function
}

fn emit_enum_global_initializers(
    function: &mut Function,
    program: &Program,
    lowering: &SettingsContext<'_>,
    debug: Option<DebugEmission<'_>>,
) {
    for variable in program.globals.iter().filter(|variable| {
        variable.value.is_some() && lowering.wasm_ir.contains_global(variable.id)
    }) {
        let value = variable.value.as_ref().unwrap();
        let ty = value_type(variable.id, lowering.semantics);
        if !ty.is_enum(lowering.standard_library) {
            continue;
        }
        let wasm_ir::ExpressionKind::Enum { variant, .. } = &lowering
            .wasm_ir
            .expression(value.id)
            .expect("global enum initializer belongs to Wasm IR")
            .kind
        else {
            unreachable!("checked enum globals use enum constructors")
        };
        if let Some(debug) = debug {
            debug.mark(
                function,
                lowering
                    .wasm_ir
                    .expression(value.id)
                    .and_then(|expression| expression.source),
            );
        }
        match (ty, variant) {
            (Type::Enum(enumeration), ResolvedEnumVariantId::Source(variant)) => {
                emit_enum_variant(
                    function,
                    enumeration,
                    *variant,
                    lowering.enums,
                    lowering.semantics,
                    lowering.gc,
                );
            }
            (Type::Standard(enumeration), ResolvedEnumVariantId::Standard(variant)) => {
                emit_standard_enum_variant(
                    function,
                    enumeration,
                    *variant,
                    lowering.standard_library,
                    lowering.gc,
                )
            }
            _ => unreachable!("checked enum globals use variants from the same declaration"),
        }
        function.instruction(&Instruction::GlobalSet(lowering.globals[&variable.id]));
    }
}

/// Materializes source aggregate constants once in the module start function.
///
/// Wasm constant expressions cannot construct GC arrays or structs. Their
/// globals therefore begin as nullable references and are populated before any
/// exported script entry point can observe them. Source expressions still see
/// the ordinary non-optional array or record type.
fn emit_aggregate_global_initializers(
    function: &mut Function,
    program: &Program,
    lowering: &EmissionContext<'_>,
    debug: Option<DebugEmission<'_>>,
) {
    let locals = HashMap::new();
    let matches = MatchLayout::default();
    let context = ExprContext {
        standard_library: lowering.standard_library,
        abi: lowering.abi,
        state: lowering.state,
        locals: LocalStorage::Wasm {
            values: &locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        intrinsic_futures: lowering.intrinsic_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        records: lowering.records,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        signatures: lowering.signatures,
        matches: &matches,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        async_frames: lowering.async_frames,
        intrinsic_capture: None,
        debug,
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };

    for variable in program.globals.iter().filter(|variable| {
        variable.value.is_some() && lowering.wasm_ir.contains_global(variable.id)
    }) {
        let value = variable.value.as_ref().unwrap();
        let ty = value_type(variable.id, lowering.semantics);
        if !matches!(
            ty,
            Type::Record(_)
                | Type::Array(_)
                | Type::Range(_)
                | Type::Set(_)
                | Type::Standard(StdlibTypeId::String)
        ) {
            continue;
        }
        compile_expr(function, value.id, &context);
        function.instruction(&Instruction::GlobalSet(lowering.globals[&variable.id]));
    }
}

fn emit_standard_enum_variant(
    function: &mut Function,
    enumeration: StdlibTypeId,
    variant: crate::stdlib::StdlibVariantId,
    standard_library: &StandardLibrary,
    gc: &GcLayout,
) {
    let selected = standard_library
        .variants_of(enumeration)
        .position(|candidate| candidate.id == variant)
        .expect("checked standard enum variants belong to their declaration");
    function.instruction(&Instruction::I32Const(selected as i32));
    for _ in standard_library.variants_of(enumeration) {
        function.instruction(&Instruction::I32Const(0));
    }
    function.instruction(&Instruction::StructNew(gc.standard_index(enumeration)));
}

/// Constructs source-language state defaults instead of relying on Wasm's
/// `struct.new_default`, because nested records are non-null source values.
fn emit_initial_state(
    function: &mut Function,
    program: &Program,
    semantics: &SemanticModel,
    gc: &GcLayout,
) {
    for field in semantics.state_storage_fields() {
        let ty = semantics
            .value_type(*field)
            .expect("checked state fields have semantic types");
        emit_source_default(
            function,
            ty,
            &program.records,
            semantics,
            gc,
            &mut Vec::new(),
        );
    }
    function.instruction(&Instruction::StructNew(STATE_TYPE));
}

fn emit_source_default(
    function: &mut Function,
    ty: TypeId,
    records: &[RecordDecl],
    semantics: &SemanticModel,
    gc: &GcLayout,
    visiting: &mut Vec<RecordId>,
) {
    let TypeKind::Record(record) = semantics.types().kind(ty) else {
        emit_default(function, semantic_type(ty, semantics), gc);
        return;
    };

    if visiting.contains(record) {
        emit_default(function, Type::Record(*record), gc);
        return;
    }

    visiting.push(*record);
    let declaration = records
        .iter()
        .find(|declaration| declaration.id == *record)
        .expect("semantic record types belong to source declarations");
    for field in &declaration.fields {
        let field_type = semantics
            .record_field_type(field.id)
            .expect("checked record fields have semantic types");
        emit_source_default(function, field_type, records, semantics, gc, visiting);
    }
    visiting.pop();
    function.instruction(&Instruction::StructNew(gc.index(Type::Record(*record))));
}
