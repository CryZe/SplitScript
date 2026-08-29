//! One-time module initialization after all backend plans are fixed.

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction};

use crate::{
    abi::AbiImportId,
    ast::{Program, RecordDecl, RecordId, ValueId},
    semantic::SemanticModel,
    types::{TypeId, TypeKind},
};

use super::{
    GcLayout, STATE_TYPE, SettingStorage, Type,
    context::EmissionContext,
    data_plan::StringPool,
    debug_artifacts::DebugEmission,
    emit_default,
    expression::{BareReturn, ExprContext, LocalStorage, MatchLayout, compile_block},
    is_wasm_global_constant,
    script_functions::{LocalPlanOptions, plan_wasm_locals},
    semantic_type,
    settings::{SettingsContext, emit_setting_registration},
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
    let mut locals = HashMap::new();
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    for initializer in emission
        .wasm_ir
        .global_initializer_plans()
        .filter(|initializer| {
            let ty = value_type(initializer.value, semantics);
            ty.has_runtime_value()
                && !is_wasm_global_constant(initializer.expression, emission.wasm_ir)
        })
    {
        plan_wasm_locals(
            &initializer.locals,
            &mut locals,
            &mut matches,
            &mut local_types,
            LocalPlanOptions {
                parameter_count: 0,
                semantics,
                wasm_ir: emission.wasm_ir,
                gc: settings_context.gc,
                reachability: emission.reachability,
                instance: None,
                include_values: true,
            },
        );
    }
    let mut function = Function::new(local_types.into_iter().map(|ty| (1, ty)));
    let debug = emission.debug_emission(start_functions.start);
    emit_runtime_global_initializers(&mut function, emission, &locals, &matches, debug);
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

/// Materializes non-Wasm-constant source initializers once in the module start
/// function.
///
/// Wasm constant expressions cannot call pure helpers or construct GC values.
/// Their globals therefore begin with backend defaults and are populated before
/// any exported script entry point can observe them.
fn emit_runtime_global_initializers(
    function: &mut Function,
    lowering: &EmissionContext<'_>,
    locals: &HashMap<ValueId, (u32, Type)>,
    matches: &MatchLayout,
    debug: Option<DebugEmission<'_>>,
) {
    let context = ExprContext {
        standard_library: lowering.standard_library,
        reachability: lowering.reachability,
        abi: lowering.abi,
        state: lowering.state,
        locals: LocalStorage::Wasm {
            values: locals,
            temporaries: &matches.temporaries,
        },
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        state_candidate: None,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        closures: lowering.closures,
        function_values: lowering.function_values,
        closure_polls: lowering.closure_polls,
        closure_environment: None,
        leaf_futures: lowering.leaf_futures,
        display_functions: lowering.display_functions,
        equality_functions: lowering.equality_functions,
        array_functions: lowering.array_functions,
        set_functions: lowering.set_functions,
        records: lowering.records,
        managed: lowering.managed,
        managed_state_reads: lowering.managed_state_reads,
        managed_state_read_functions: lowering.managed_state_read_functions,
        managed_snapshot_functions: lowering.managed_snapshot_functions,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        signatures: lowering.signatures,
        matches,
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

    for initializer in lowering.wasm_ir.global_initializer_plans() {
        let ty = value_type(initializer.value, lowering.semantics);
        if !ty.has_runtime_value()
            || is_wasm_global_constant(initializer.expression, lowering.wasm_ir)
        {
            continue;
        }
        compile_block(function, &initializer.entry, &context, None);
    }
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
