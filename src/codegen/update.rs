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
    global_plan::RuntimeGlobals, imports::Abi, semantic_type, state_storage_index, value_type,
};

/// Per-tick runtime view of the completed backend plans.
pub(super) struct UpdateContext<'a> {
    pub standard_library: &'a crate::stdlib::StandardLibrary,
    pub abi: &'a Abi,
    pub gc: &'a GcLayout,
    pub runtime_globals: RuntimeGlobals,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub process_names: &'a [&'a str],
    pub provider_attach: Option<ProviderAttach>,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderAttach {
    pub init: u32,
    pub poll: u32,
    pub frame_global: u32,
    pub frame_type: u32,
    pub completion_field: u32,
}

pub(super) struct StatePollFunctions<'a> {
    pub reads: &'a [u32],
    pub transforms: &'a [Option<u32>],
}

pub(super) fn compile_update(
    program: &Program,
    strings: &StringPool,
    state_functions: StatePollFunctions<'_>,
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
    let all_fields = state.all_fields().collect::<Vec<_>>();
    for field in &all_fields {
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
    for (process_index, process) in lowering.process_names.iter().enumerate() {
        let (process_ptr, process_len) = strings.get(process);
        function
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(process_ptr as i32))
            .instruction(&Instruction::I32Const(process_len as i32))
            .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessAttach)))
            .instruction(&Instruction::GlobalSet(globals.process))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::I32Const(process_index as i32))
            .instruction(&Instruction::GlobalSet(globals.process_name))
            .instruction(&Instruction::End)
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
        .instruction(&Instruction::GlobalSet(globals.process))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::GlobalSet(globals.process_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::GlobalSet(globals.state_ready));
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
    if let (Some(frame_global), Some(ProviderAttach { frame_type, .. })) =
        (globals.provider_attachment_frame, lowering.provider_attach)
    {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(frame_type)))
            .instruction(&Instruction::GlobalSet(frame_global));
    }
    if let (Some(selected), Some(enumeration)) =
        (globals.selected_layout, state.layout_enum.as_ref())
    {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                lowering.gc.index(Type::Enum(enumeration.id)),
            )))
            .instruction(&Instruction::GlobalSet(selected));
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
        function.instruction(&Instruction::If(BlockType::Empty));
        let ProviderAttach {
            init,
            poll,
            frame_global,
            frame_type,
            completion_field,
        } = provider_attach;
        function
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(init))
            .instruction(&Instruction::GlobalSet(frame_global))
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::Call(poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: frame_type,
                field_index: completion_field,
            })
            .instruction(&Instruction::GlobalSet(provider_global))
            .instruction(&Instruction::RefNull(HeapType::Concrete(frame_type)))
            .instruction(&Instruction::GlobalSet(frame_global));
        function.instruction(&Instruction::End);
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
    if state.layouts.is_empty() {
        for (read_index, field) in all_fields.iter().enumerate() {
            emit_state_field_poll(
                &mut function,
                field.id,
                state_functions.reads[read_index],
                state_functions.transforms[read_index],
                first_poll_result + read_index as u32,
                candidate_state,
                lowering,
            );
        }
    } else {
        let selected = globals
            .selected_layout
            .expect("named layouts have selected-layout storage");
        let enumeration = state
            .layout_enum
            .as_ref()
            .expect("named layouts generate a typed enum");
        for (layout_index, layout) in state.layouts.iter().enumerate() {
            function
                .instruction(&Instruction::GlobalGet(selected))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: lowering.gc.index(Type::Enum(enumeration.id)),
                    field_index: 0,
                })
                .instruction(&Instruction::I32Const(layout_index as i32))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty));
            for field in &layout.fields {
                let read_index = all_fields
                    .iter()
                    .position(|candidate| candidate.id == field.id)
                    .expect("layout fields belong to the read-function plan");
                emit_state_field_poll(
                    &mut function,
                    field.id,
                    state_functions.reads[read_index],
                    state_functions.transforms[read_index],
                    first_poll_result + read_index as u32,
                    candidate_state,
                    lowering,
                );
            }
            function.instruction(&Instruction::End);
        }
    }
    function
        .instruction(&Instruction::GlobalGet(globals.state_ready))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.current))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.old))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::GlobalSet(globals.state_ready))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
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

fn emit_state_field_poll(
    function: &mut Function,
    field: crate::ast::ValueId,
    read_function: u32,
    transform_function: Option<u32>,
    poll_result_local: u32,
    candidate_state: u32,
    lowering: &UpdateContext<'_>,
) {
    let semantics = lowering.semantics;
    let Type::Result(result_type) = semantic_type(
        semantics
            .state_poll_result(field)
            .expect("checked state fields have poll-result types"),
        semantics,
    ) else {
        unreachable!("state poll-result types are Result layouts")
    };
    let (field_index, _) = state_storage_index(field, semantics);
    let field_type = value_type(field, semantics);
    function
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
        .instruction(&Instruction::Call(read_function))
        .instruction(&Instruction::LocalSet(poll_result_local));

    // A filter is itself fallible. A failed raw read bypasses it so the
    // original error reaches the field's acceptance boundary unchanged.
    if let Some(transform) = transform_function {
        function
            .instruction(&Instruction::LocalGet(poll_result_local))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.index(Type::Result(result_type)),
                field_index: 1,
            })
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Else);
        emit_poll_value(
            function,
            poll_result_local,
            result_type,
            field_type,
            lowering,
        );
        function
            .instruction(&Instruction::Call(transform))
            .instruction(&Instruction::LocalSet(poll_result_local))
            .instruction(&Instruction::End);
    }

    // Before initialization every required field must succeed in the same
    // poll. Afterwards, an error retains this field's accepted value while
    // successful sibling fields continue building the candidate snapshot.
    function
        .instruction(&Instruction::LocalGet(poll_result_local))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: lowering.gc.index(Type::Result(result_type)),
            field_index: 1,
        })
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::GlobalGet(
            lowering.runtime_globals.state_ready,
        ))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.current))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, STATE_TYPE, field_index, field_type);
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: STATE_TYPE,
            field_index,
        })
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::RefAsNonNull);
    emit_poll_value(
        function,
        poll_result_local,
        result_type,
        field_type,
        lowering,
    );
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: STATE_TYPE,
            field_index,
        })
        .instruction(&Instruction::End);
}

fn emit_poll_value(
    function: &mut Function,
    poll_result_local: u32,
    result_type: crate::ast::ResultTypeId,
    field_type: Type,
    lowering: &UpdateContext<'_>,
) {
    function
        .instruction(&Instruction::LocalGet(poll_result_local))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        lowering.gc.index(Type::Result(result_type)),
        0,
        field_type,
    );
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
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::GlobalSet(globals.state_ready))
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
