//! Per-tick process, snapshot, lifecycle-action, and timer runtime emission.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, RefType, ValType};

use crate::{
    abi::AbiImportId,
    ast::{ActionKind, Program, ValueId},
    stdlib::{CoreTypeId, RuntimeRepresentation, StdlibFieldId, StdlibTypeId},
    wasm_ir,
};

use super::{
    GcLayout, STATE_TYPE, Type,
    data_plan::StringPool,
    emit_typed_struct_get,
    global_plan::RuntimeGlobals,
    imports::Abi,
    managed_state_reads::ManagedStateReadCache,
    memory_plan::AbiReadScratch,
    pointer_prefixes::{
        PointerPrefixPlan, PrefixEmissionContext, PrefixEmissionState, PrefixLocals,
    },
    record_field_type, semantic_type, state_storage_index, value_type,
};

const ATTACH_READY: i32 = 1;
const ATTACH_REJECTED: i32 = 2;
const ATTACH_LAYOUT_SELECTED: i32 = 3;

/// Per-tick runtime view of the completed backend plans.
pub(super) struct UpdateContext<'a> {
    pub standard_library: &'a crate::stdlib::StandardLibrary,
    pub abi: &'a Abi,
    pub gc: &'a GcLayout,
    pub runtime_globals: RuntimeGlobals,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub managed: &'a crate::managed::ManagedBindingPlan,
    pub managed_state_reads: &'a ManagedStateReadCache,
    pub pointer_prefixes: &'a PointerPrefixPlan,
    pub abi_read: AbiReadScratch,
    pub explicit_layout_selection: bool,
    pub globals: &'a HashMap<ValueId, u32>,
    pub global_types: &'a HashMap<ValueId, Type>,
    pub attachment_globals: &'a [ValueId],
    pub process_names: &'a [&'a str],
    pub provider_attach: Option<ProviderAttach>,
    pub provider_preparation: Option<ProviderPreparation>,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderAttach {
    pub init: u32,
    pub poll: u32,
    pub frame_global: u32,
    pub frame_type: u32,
    pub completion_field: u32,
}

#[derive(Clone, Copy)]
pub(super) struct ProviderPreparation {
    pub init: u32,
    pub poll: u32,
    pub frame_global: u32,
    pub frame_type: u32,
    pub completion_field: u32,
    pub value_global: u32,
    pub value_type: Type,
    pub ready_global: u32,
}

pub(super) struct StatePollFunctions<'a> {
    pub reads: &'a [u32],
    pub transforms: &'a [Option<u32>],
}

#[derive(Clone, Copy)]
struct StateFieldPoll {
    field: ValueId,
    read_function: u32,
    transform_function: Option<u32>,
    poll_result_local: u32,
}

struct SnapshotPollContext<'a> {
    candidate_state: u32,
    pointer_prefix_locals: &'a PrefixLocals,
    pointer_emission: PrefixEmissionContext<'a>,
    lowering: &'a UpdateContext<'a>,
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
    let timer_state = 0;
    let nullable_bool = 1;
    let newly_attached = 2;
    let duration_local = 3;
    let candidate_state = if has_game_time { 4 } else { 3 };
    let first_poll_result = candidate_state + 1;
    let mut locals = vec![(3, ValType::I32)];
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
    let first_prefix_local = first_poll_result + all_fields.len() as u32;
    let (prefix_local_declarations, pointer_prefix_locals) = lowering
        .pointer_prefixes
        .allocate_locals(first_prefix_local);
    locals.extend(prefix_local_declarations);
    let mut function = Function::new(locals);
    let snapshot_poll = SnapshotPollContext {
        candidate_state,
        pointer_prefix_locals: &pointer_prefix_locals,
        pointer_emission: PrefixEmissionContext {
            plan: lowering.pointer_prefixes,
            strings,
            abi: lowering.abi,
            process_global: lowering.runtime_globals.process,
            abi_read: lowering.abi_read,
        },
        lowering,
    };

    if let Some(refresh_settings) = refresh_settings {
        function.instruction(&Instruction::Call(refresh_settings));
    }
    function
        .instruction(&Instruction::GlobalGet(globals.process))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
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
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::LocalSet(newly_attached))
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
        .instruction(&Instruction::LocalGet(newly_attached))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::F64Const(program.attached_tick_rate().into()))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimeSetTickRate),
        ))
        .instruction(&Instruction::End)
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
    if let Some(preparation) = lowering.provider_preparation {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                preparation.frame_type,
            )))
            .instruction(&Instruction::GlobalSet(preparation.frame_global));
        emit_storage_default(&mut function, lowering.gc.val_type(preparation.value_type));
        function
            .instruction(&Instruction::GlobalSet(preparation.value_global))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(preparation.ready_global));
    }
    if let (Some(selected), Some(layout_value)) = (globals.selected_layout, state.layout_value) {
        let layout_type = lowering.global_types[&layout_value];
        emit_storage_default(&mut function, lowering.gc.val_type(layout_type));
        function.instruction(&Instruction::GlobalSet(selected));
    }
    for value in lowering.attachment_globals {
        let ty = lowering.global_types[value];
        if !ty.has_runtime_value() {
            continue;
        }
        emit_storage_default(&mut function, lowering.gc.val_type(ty));
        function.instruction(&Instruction::GlobalSet(lowering.globals[value]));
    }
    if let Some(region) = cancellation_region {
        emit_cancel_region(&mut function, region, lowering.gc, globals);
    }
    function
        .instruction(&Instruction::F64Const(program.detached_tick_rate().into()))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimeSetTickRate),
        ));
    if let Some(detach) = actions.get(&ActionKind::OnDetach) {
        function.instruction(&Instruction::Call(*detach));
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

    if let Some(preparation) = lowering.provider_preparation {
        function
            .instruction(&Instruction::GlobalGet(preparation.ready_global))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Call(preparation.init))
            .instruction(&Instruction::GlobalSet(preparation.frame_global))
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::Call(preparation.poll))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::GlobalGet(preparation.frame_global))
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: preparation.frame_type,
                field_index: preparation.completion_field,
            })
            .instruction(&Instruction::GlobalSet(preparation.value_global))
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                preparation.frame_type,
            )))
            .instruction(&Instruction::GlobalSet(preparation.frame_global))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(preparation.ready_global))
            .instruction(&Instruction::End);
    }

    let automatic_layout = if lowering.explicit_layout_selection {
        None
    } else {
        lowering.managed.automatic_layout.as_ref().filter(|plan| {
            plan.evidence_fields.is_empty()
                || semantics.state_provider() == Some(crate::stdlib::StdlibStateProviderId::Unity)
        })
    };
    if let Some(plan) = automatic_layout {
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_automatic_layout_selection(&mut function, program, plan, lowering);
        function
            .instruction(&Instruction::GlobalGet(
                globals
                    .selected_layout
                    .expect("automatic layout selection has global storage"),
            ))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(ATTACH_REJECTED))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::I32Const(
                if actions.contains_key(&ActionKind::OnAttach) {
                    ATTACH_LAYOUT_SELECTED
                } else {
                    ATTACH_READY
                },
            ))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::End);
        if let Some(on_attach) = actions.get(&ActionKind::OnAttach) {
            function
                .instruction(&Instruction::GlobalGet(globals.attach_ready))
                .instruction(&Instruction::I32Const(ATTACH_LAYOUT_SELECTED))
                .instruction(&Instruction::I32Eq)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::GlobalGet(globals.process))
                .instruction(&Instruction::Call(*on_attach))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End)
                .instruction(&Instruction::I32Const(ATTACH_READY))
                .instruction(&Instruction::GlobalSet(globals.attach_ready))
                .instruction(&Instruction::End);
        }
    } else if let Some(on_attach) = actions.get(&ActionKind::OnAttach) {
        function
            .instruction(&Instruction::GlobalGet(globals.attach_ready))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::GlobalGet(globals.process))
            .instruction(&Instruction::Call(*on_attach))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
        emit_managed_field_presence_validation(&mut function, program, lowering);
        function
            .instruction(&Instruction::I32Const(ATTACH_READY))
            .instruction(&Instruction::GlobalSet(globals.attach_ready))
            .instruction(&Instruction::End);
    }

    // `2` records that metadata evidence contradicted the layout chosen by
    // `onAttach`. Keep the process attached so the normal process-lifetime
    // boundary resets everything when it exits, without repeatedly invoking
    // user attachment code or polling an invalid schema.
    function
        .instruction(&Instruction::GlobalGet(globals.attach_ready))
        .instruction(&Instruction::I32Const(ATTACH_REJECTED))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    // Managed static fields are attachment metadata roots, but their values
    // (notably Unity singleton instances) may be replaced while the process
    // remains alive. Share each read only within this snapshot attempt and
    // clear the transaction before any field is polled.
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::GlobalSet(active));
    }
    for storage in lowering.managed_state_reads.entries() {
        function
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                lowering.gc.index(Type::Result(storage.result)),
            )))
            .instruction(&Instruction::GlobalSet(storage.global));
    }

    function
        .instruction(&Instruction::StructNewDefault(STATE_TYPE))
        .instruction(&Instruction::LocalSet(candidate_state));
    if state.layouts.is_empty() {
        let mut prefix_emission = PrefixEmissionState::default();
        for (read_index, field) in all_fields.iter().enumerate() {
            let predicate = semantics.state_field_layout_predicate(field.id);
            if let Some(predicate) = predicate {
                emit_layout_predicate(&mut function, program, predicate, lowering);
                function.instruction(&Instruction::If(BlockType::Empty));
            }
            emit_state_field_poll(
                &mut function,
                StateFieldPoll {
                    field: field.id,
                    read_function: state_functions.reads[read_index],
                    transform_function: state_functions.transforms[read_index],
                    poll_result_local: first_poll_result + read_index as u32,
                },
                &mut prefix_emission,
                predicate.is_some(),
                &snapshot_poll,
            );
            if predicate.is_some() {
                function.instruction(&Instruction::End);
            }
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
            let mut prefix_emission = PrefixEmissionState::default();
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
                    StateFieldPoll {
                        field: field.id,
                        read_function: state_functions.reads[read_index],
                        transform_function: state_functions.transforms[read_index],
                        poll_result_local: first_poll_result + read_index as u32,
                    },
                    &mut prefix_emission,
                    false,
                    &snapshot_poll,
                );
            }
            function.instruction(&Instruction::End);
        }
    }
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(active));
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
        .instruction(&Instruction::GlobalSet(globals.state_ready));
    if let Some(on_state_ready) = actions.get(&ActionKind::OnStateReady) {
        emit_action_args(&mut function, globals);
        function.instruction(&Instruction::Call(*on_state_ready));
    }
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::GlobalGet(globals.current))
        .instruction(&Instruction::GlobalSet(globals.old))
        .instruction(&Instruction::LocalGet(candidate_state))
        .instruction(&Instruction::GlobalSet(globals.current));

    if let Some(update) = actions.get(&ActionKind::WhileAttached) {
        emit_action_args(&mut function, globals);
        function
            .instruction(&Instruction::Call(*update))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
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

fn emit_layout_predicate(
    function: &mut Function,
    program: &Program,
    predicate: &crate::semantic::ResolvedLayoutPredicate,
    lowering: &UpdateContext<'_>,
) {
    for (alternative_index, alternative) in predicate.alternatives.iter().enumerate() {
        emit_layout_constraints(function, program, alternative, lowering);
        if alternative_index != 0 {
            function.instruction(&Instruction::I32Or);
        }
    }
    if predicate.alternatives.is_empty() {
        function.instruction(&Instruction::I32Const(0));
    }
}

fn emit_layout_constraints(
    function: &mut Function,
    program: &Program,
    constraints: &[crate::semantic::ResolvedLayoutConstraint],
    lowering: &UpdateContext<'_>,
) {
    let state = program
        .state
        .as_ref()
        .expect("state polling has a state declaration");
    let layout = state
        .layout
        .as_ref()
        .expect("conditional fields require attachment layout dimensions");
    let record = program
        .records
        .get(layout.record.index())
        .expect("attachment Layout is an ordinary record");
    let selected = lowering
        .runtime_globals
        .selected_layout
        .expect("conditional fields have selected-layout storage");
    for (index, constraint) in constraints.iter().enumerate() {
        let field_index = record
            .fields
            .iter()
            .position(|field| field.id == constraint.dimension)
            .expect("layout constraints refer to Layout fields") as u32;
        let field_type = semantic_type(
            lowering
                .semantics
                .record_field_type(constraint.dimension)
                .expect("layout dimensions have checked enum types"),
            lowering.semantics,
        );
        let Type::Enum(enumeration) = field_type else {
            unreachable!("validated layout dimensions are source enums")
        };
        let enumeration_decl = program
            .enum_declarations()
            .find(|candidate| candidate.id == enumeration)
            .expect("layout dimension enums belong to the source program");
        let variant_index = enumeration_decl
            .variants
            .iter()
            .position(|variant| variant.id == constraint.variant)
            .expect("layout constraints refer to variants of their dimension")
            as i32;

        function
            .instruction(&Instruction::GlobalGet(selected))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            function,
            lowering.gc.index(Type::Record(layout.record)),
            field_index,
            field_type,
        );
        function
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: lowering.gc.index(field_type),
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(variant_index))
            .instruction(&Instruction::I32Eq);
        if index != 0 {
            function.instruction(&Instruction::I32And);
        }
    }
}

fn emit_managed_field_presence_validation(
    function: &mut Function,
    program: &Program,
    lowering: &UpdateContext<'_>,
) {
    use crate::stdlib::{MANAGED_BINDINGS_TYPE, managed_field_presence_name};

    let Some(bindings_global) = lowering.runtime_globals.provider_preparation_value else {
        return;
    };
    let Some(bindings) = program
        .records
        .iter()
        .find(|record| record.name == MANAGED_BINDINGS_TYPE)
    else {
        return;
    };

    for class in &lowering.managed.classes {
        for group in &class.conditional_fields {
            emit_layout_predicate(function, program, &group.predicate, lowering);
            function.instruction(&Instruction::If(BlockType::Empty));
            for field in &group.fields {
                let name = managed_field_presence_name(field.id.index());
                let (field_index, presence) = bindings
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, candidate)| candidate.name == name)
                    .expect("conditional managed fields have generated presence storage");
                let presence_type = record_field_type(presence.id, lowering.semantics);
                function
                    .instruction(&Instruction::GlobalGet(bindings_global))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(
                    function,
                    lowering.gc.index(Type::Record(bindings.id)),
                    field_index as u32,
                    presence_type,
                );
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(ATTACH_REJECTED))
                    .instruction(&Instruction::GlobalSet(
                        lowering.runtime_globals.attach_ready,
                    ))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            function.instruction(&Instruction::End);
        }
    }
}

fn emit_automatic_layout_selection(
    function: &mut Function,
    program: &Program,
    plan: &crate::layout_selection::LayoutSelectionPlan,
    lowering: &UpdateContext<'_>,
) {
    use crate::stdlib::{MANAGED_BINDINGS_TYPE, managed_field_presence_name};

    let bindings = (!plan.evidence_fields.is_empty()).then(|| {
        let global = lowering
            .runtime_globals
            .provider_preparation_value
            .expect("managed layout evidence has provider preparation storage");
        let record = program
            .records
            .iter()
            .find(|record| record.name == MANAGED_BINDINGS_TYPE)
            .expect("managed layout evidence has generated bindings");
        (global, record)
    });
    let layout = program
        .state
        .as_ref()
        .and_then(|state| state.layout.as_ref())
        .expect("automatic selection has attachment layout dimensions");
    let selected = lowering
        .runtime_globals
        .selected_layout
        .expect("automatic selection has layout storage");

    for candidate in &plan.candidates {
        if let Some((bindings_global, bindings_record)) = bindings {
            for (index, field) in plan.evidence_fields.iter().enumerate() {
                let name = managed_field_presence_name(field.index());
                let (field_index, declaration) = bindings_record
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, candidate)| candidate.name == name)
                    .expect("layout evidence has generated presence storage");
                let field_type = record_field_type(declaration.id, lowering.semantics);
                function
                    .instruction(&Instruction::GlobalGet(bindings_global))
                    .instruction(&Instruction::RefAsNonNull);
                emit_typed_struct_get(
                    function,
                    lowering.gc.index(Type::Record(bindings_record.id)),
                    field_index as u32,
                    field_type,
                );
                if !candidate.present_fields.contains(field) {
                    function.instruction(&Instruction::I32Eqz);
                }
                if index != 0 {
                    function.instruction(&Instruction::I32And);
                }
            }
        } else {
            function.instruction(&Instruction::I32Const(1));
        }
        function.instruction(&Instruction::If(BlockType::Empty));
        for (dimension, variant) in plan.dimensions.iter().zip(&candidate.variants) {
            let enumeration = program
                .enum_declaration(dimension.enumeration)
                .expect("layout dimension enum belongs to the source program");
            let variant_index = enumeration
                .variants
                .iter()
                .position(|candidate| candidate.id == *variant)
                .expect("layout candidate uses a declared variant");
            function.instruction(&Instruction::I32Const(variant_index as i32));
            for _ in &enumeration.variants {
                function.instruction(&Instruction::I32Const(0));
            }
            function.instruction(&Instruction::StructNew(
                lowering.gc.index(Type::Enum(enumeration.id)),
            ));
        }
        function
            .instruction(&Instruction::StructNew(
                lowering.gc.index(Type::Record(layout.record)),
            ))
            .instruction(&Instruction::GlobalSet(selected))
            .instruction(&Instruction::End);
    }
}

/// Clears attachment-owned storage without constructing source defaults.
/// Reference-typed globals are nullable at the storage boundary precisely so
/// a detached runtime cannot retain GC objects from the previous process.
fn emit_storage_default(function: &mut Function, ty: ValType) {
    match ty {
        ValType::I32 => function.instruction(&Instruction::I32Const(0)),
        ValType::I64 => function.instruction(&Instruction::I64Const(0)),
        ValType::F32 => function.instruction(&Instruction::F32Const(0.0.into())),
        ValType::F64 => function.instruction(&Instruction::F64Const(0.0.into())),
        ValType::V128 => function.instruction(&Instruction::V128Const(0)),
        ValType::Ref(reference) => function.instruction(&Instruction::RefNull(reference.heap_type)),
    };
}

fn emit_state_field_poll(
    function: &mut Function,
    poll: StateFieldPoll,
    prefix_emission: &mut PrefixEmissionState,
    conditional: bool,
    context: &SnapshotPollContext<'_>,
) {
    let StateFieldPoll {
        field,
        read_function,
        transform_function,
        poll_result_local,
    } = poll;
    let lowering = context.lowering;
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
    function.instruction(&Instruction::GlobalGet(lowering.runtime_globals.process));
    if let Some(prefix) = lowering.pointer_prefixes.field(field) {
        context.pointer_prefix_locals.emit_field_prefix(
            function,
            prefix,
            &context.pointer_emission,
            prefix_emission,
            conditional,
        );
    }
    function
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
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some(active) = lowering.managed_state_reads.active() {
        function
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::GlobalSet(active));
    }
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(context.candidate_state))
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
        .instruction(&Instruction::LocalGet(context.candidate_state))
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
