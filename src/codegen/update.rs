//! Per-tick process, snapshot, lifecycle-action, and timer runtime emission.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, RefType, ValType};

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program},
    stdlib::{CoreTypeId, RuntimeRepresentation, StdlibFieldId, StdlibTypeId},
    wasm_ir,
};

use super::{
    GcLayout, STATE_TYPE, Type, data_plan::StringPool, emit_typed_struct_get,
    global_plan::RuntimeGlobals, imports::Abi, semantic_type, value_type,
};

/// Per-tick runtime view of the completed backend plans.
pub(super) struct UpdateContext<'a> {
    pub standard_library: &'a crate::stdlib::StandardLibrary,
    pub abi: &'a Abi,
    pub gc: &'a GcLayout,
    pub runtime_globals: RuntimeGlobals,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub process_names: &'a [&'a str],
    pub provider_attach: Option<u32>,
}

pub(super) fn compile_update(
    program: &Program,
    strings: &StringPool,
    read_functions: &[u32],
    actions: &HashMap<ActionKind, u32>,
    refresh_settings: Option<u32>,
    cancellation_region: Option<wasm_ir::CancellationRegion>,
    lowering: &UpdateContext<'_>,
) -> Function {
    let abi = lowering.abi;
    let globals = lowering.runtime_globals;
    let semantics = lowering.semantics;
    let has_game_time = actions.contains_key(&ActionKind::GameTime);
    let mut locals = vec![(2, ValType::I32)];
    if has_game_time {
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(lowering.gc.standard_index(StdlibTypeId::Duration)),
            }),
        ));
    }
    locals.push((
        1,
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(STATE_TYPE),
        }),
    ));
    let state = program.state.as_ref().unwrap();
    for field in &state.fields {
        let poll_result = semantic_type(
            semantics
                .state_poll_result(field.id)
                .expect("checked state fields have poll-result types"),
            semantics,
        );
        locals.push((1, lowering.gc.val_type(poll_result)));
    }
    let mut function = Function::new(locals);
    let timer_state = 0;
    let nullable_bool = 1;
    let duration_local = 2;
    let candidate_state = if has_game_time { 3 } else { 2 };
    let first_poll_result = candidate_state + 1;

    if let Some(refresh_settings) = refresh_settings {
        function.instruction(&Instruction::Call(refresh_settings));
    }
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some(detached) = actions.get(&ActionKind::OnDetached) {
        function
            .instruction(&Instruction::GlobalGet(globals.detached_entered))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*detached))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(globals.detached_entered))
            .instruction(&Instruction::End);
    }
    for process in lowering.process_names {
        let (process_ptr, process_len) = strings.get(process);
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(process_ptr as i32))
            .instruction(&Instruction::I32Const(process_len as i32))
            .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessAttach)))
            .instruction(&Instruction::GlobalSet(globals.process))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::GlobalSet(globals.detached_entered))
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessIsOpen)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessDetach)))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::GlobalSet(globals.process));
    if let Some(provider_global) = globals.provider_value {
        let provider_type = semantics
            .state_provider()
            .map(|provider| {
                lowering
                    .standard_library
                    .state_provider(provider)
                    .process_type
            })
            .expect("provider storage requires a resolved provider");
        emit_provider_default(&mut function, provider_type, lowering);
        function.instruction(&Instruction::GlobalSet(provider_global));
    }
    if let Some(region) = cancellation_region {
        emit_cancel_region(&mut function, region, lowering.gc, globals);
    }
    if let Some(detached) = actions.get(&ActionKind::OnDetached) {
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*detached))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(globals.detached_entered));
    }
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if let (Some(provider_global), Some(provider_attach)) =
        (globals.provider_value, lowering.provider_attach)
    {
        let provider_type = semantics
            .state_provider()
            .map(|provider| {
                lowering
                    .standard_library
                    .state_provider(provider)
                    .process_type
            })
            .expect("provider storage requires a resolved provider");
        emit_provider_unavailable(&mut function, provider_global, provider_type, lowering);
        function
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(globals.process));
        function.instruction(&Instruction::Call(provider_attach));
        function
            .instruction(&Instruction::GlobalSet(provider_global))
            .instruction(&Instruction::End);
        emit_provider_unavailable(&mut function, provider_global, provider_type, lowering);
        function
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }

    if let Some(on_attach) = actions.get(&ActionKind::OnAttach) {
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::Call(*on_attach))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::End);
    }

    function
        .instruction(&Instruction::StructNewDefault(STATE_TYPE))
        .instruction(&Instruction::LocalSet(candidate_state));
    for (index, field) in state.fields.iter().enumerate() {
        let Type::Result(result_type) = semantic_type(
            semantics
                .state_poll_result(field.id)
                .expect("checked state fields have poll-result types"),
            semantics,
        ) else {
            unreachable!("state poll-result types are Result layouts")
        };
        let poll_result_local = first_poll_result + index as u32;
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::Call(read_functions[index]))
            .instruction(&Instruction::LocalTee(poll_result_local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.index(Type::Result(result_type)),
                field_index: 1,
            })
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(candidate_state))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::LocalGet(poll_result_local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            lowering.gc.index(Type::Result(result_type)),
            0,
            value_type(field.id, semantics),
        );
        function.instruction(&Instruction::StructSet {
            struct_type_index: STATE_TYPE,
            field_index: index as u32,
        });
    }
    function
        .instruction(&Instruction::GlobalGet(globals.current))
        .instruction(&Instruction::GlobalSet(globals.old))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.current));

    if let Some(update) = actions.get(&ActionKind::WhileAttached) {
        emit_action_args(&mut function, globals);
        function.instruction(&Instruction::Call(*update));
    }
    function
        .instruction(&Instruction::Call(abi.function(AbiImportId::TimerGetState)))
        .instruction(&Instruction::LocalSet(timer_state));

    if let Some(start) = actions.get(&ActionKind::Start) {
        function
            .instruction(&Instruction::LocalGet(timer_state))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*start))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(abi.function(AbiImportId::TimerStart)))
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
    }

    function
        .instruction(&Instruction::LocalGet(timer_state))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(timer_state))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));

    if let Some(is_loading) = actions.get(&ActionKind::IsLoading) {
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*is_loading))
            .instruction(&Instruction::LocalTee(nullable_bool))
            .instruction(&Instruction::I32Const(-1))
            .instruction(&Instruction::I32Ne)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::LocalGet(nullable_bool))
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::TimerPauseGameTime),
            ))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::TimerResumeGameTime),
            ))
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
    }
    if let Some(game_time) = actions.get(&ActionKind::GameTime) {
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*game_time))
            .instruction(&Instruction::LocalTee(duration_local))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::LocalGet(duration_local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.standard_index(StdlibTypeId::Duration),
                field_index: lowering
                    .gc
                    .standard_field_index(StdlibFieldId::DurationSeconds),
            })
            .instruction(&Instruction::LocalGet(duration_local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.standard_index(StdlibTypeId::Duration),
                field_index: lowering
                    .gc
                    .standard_field_index(StdlibFieldId::DurationNanoseconds),
            })
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::TimerSetGameTime),
            ))
            .instruction(&Instruction::End);
    }

    if let Some(reset) = actions.get(&ActionKind::Reset) {
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*reset))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(abi.function(AbiImportId::TimerReset)));
        if let Some(split) = actions.get(&ActionKind::Split) {
            function.instruction(&Instruction::Else);
            emit_split(&mut function, *split, abi, globals);
        }
        function.instruction(&Instruction::End);
    } else if let Some(split) = actions.get(&ActionKind::Split) {
        emit_split(&mut function, *split, abi, globals);
    }

    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

fn emit_provider_default(function: &mut Function, ty: StdlibTypeId, context: &UpdateContext<'_>) {
    match context.standard_library.type_decl(ty).representation {
        RuntimeRepresentation::Scalar {
            storage: CoreTypeId::I64,
        } => {
            function.instruction(&Instruction::I64Const(0));
        }
        RuntimeRepresentation::GcStruct { .. }
        | RuntimeRepresentation::GcArray { .. }
        | RuntimeRepresentation::Enum { .. } => {
            function.instruction(&Instruction::RefNull(HeapType::Concrete(
                context.gc.standard_index(ty),
            )));
        }
        representation => {
            unreachable!("unsupported state-provider representation: {representation:?}")
        }
    }
}

fn emit_provider_unavailable(
    function: &mut Function,
    global: u32,
    ty: StdlibTypeId,
    context: &UpdateContext<'_>,
) {
    function.instruction(&Instruction::GlobalGet(global));
    match context.standard_library.type_decl(ty).representation {
        RuntimeRepresentation::Scalar {
            storage: CoreTypeId::I64,
        } => {
            function.instruction(&Instruction::I64Eqz);
        }
        RuntimeRepresentation::GcStruct { .. }
        | RuntimeRepresentation::GcArray { .. }
        | RuntimeRepresentation::Enum { .. } => {
            function.instruction(&Instruction::RefIsNull);
        }
        representation => {
            unreachable!("unsupported state-provider representation: {representation:?}")
        }
    }
}

fn emit_cancel_region(
    function: &mut Function,
    region: wasm_ir::CancellationRegion,
    gc: &GcLayout,
    globals: RuntimeGlobals,
) {
    match region {
        wasm_ir::CancellationRegion::ProcessLifetime => {
            function
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(globals.attach_ready))
                .instruction(&Instruction::StructNewDefault(gc.async_frame_index()))
                .instruction(&Instruction::GlobalSet(globals.async_frame));
        }
    }
}

fn emit_split(function: &mut Function, split: u32, abi: &Abi, globals: RuntimeGlobals) {
    emit_action_args(function, globals);
    function
        .instruction(&Instruction::Call(split))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Call(abi.function(AbiImportId::TimerSplit)))
        .instruction(&Instruction::End);
}

fn emit_action_args(function: &mut Function, globals: RuntimeGlobals) {
    function
        .instruction(&Instruction::GlobalGet(globals.current))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::GlobalGet(globals.old))
        .instruction(&Instruction::RefAsNonNull);
}
