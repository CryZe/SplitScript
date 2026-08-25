//! Wasm-IR async state-machine, suspension, retry, and cancellation emission.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::semantic::FunctionInstance;
use crate::{
    abi::AbiImportId,
    ast::{Action, ExprId, SuspensionMode, ValueId},
    intrinsic_registry::RuntimeHelperId,
    stdlib::{IntrinsicId, StdlibFieldId, StdlibTypeId},
    types::TypeKind,
    wasm_ir::{self, BodyOwner},
};

use super::{
    LocalPlanOptions, MemoryByteOrder, Type,
    async_frame::{
        AsyncFrameLayout, AsyncFrameRef, AsyncFrameSource, IntrinsicFutureInstance,
        IntrinsicFutureLayout,
    },
    call_target,
    context::AttachContext,
    data_plan::StringPool,
    emit_default, emit_memory_value, emit_string_literal, emit_typed_struct_get,
    expression::{
        BareReturn, ExprContext, IntrinsicCapture, LocalStorage, LoopControl, MatchLayout,
        compile_assignment, compile_expr, compile_fallback_condition, compile_for_bind_and_advance,
        compile_for_has_next, compile_for_init, compile_receiver, compile_statement_pattern,
        compile_temporary_set, store_match_binding,
    },
    imports::Abi,
    memarg, plan_wasm_locals, resolved_intrinsic, semantic_type, unity_layout,
};

/// Candidate start positions inspected by one signature-future poll. At the
/// default attached rate of 120 Hz, 512 KiB provides roughly 60 MiB/s of scan
/// throughput. The range helper reads smaller pages within this window, and
/// control returns to the host between windows so large modules cannot
/// monopolize the autosplitter runtime.
const SIGNATURE_SCAN_CANDIDATES_PER_POLL: i64 = 512 * 1024;

pub(super) fn compile_async_attach(
    action: &Action,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Action(action.kind))
        .expect("checked actions have Wasm IR bodies");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.async_frame_index(),
        source: AsyncFrameSource::Global(runtime.lowering.runtime_globals.async_frame),
    };
    let result_global = runtime
        .lowering
        .state
        .layout_enum
        .is_some()
        .then_some(runtime.lowering.runtime_globals.selected_layout)
        .flatten();
    compile_async_body(
        &wasm_body.entry,
        &wasm_body.locals,
        wasm_body.async_state_count,
        wasm_body
            .cancellation_region
            .expect("onAttach has a process-lifetime cancellation region"),
        layout,
        runtime,
        frame,
        None,
        BareReturn::AsyncAttach { result_global },
        result_global,
        function_index,
    )
}

pub(super) fn compile_async_function_poll(
    instance: &FunctionInstance,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Function(instance.clone()))
        .expect("checked functions have Wasm IR bodies");
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.function_frame_index(instance),
        source: AsyncFrameSource::Local(0),
    };
    compile_async_body(
        &wasm_body.entry,
        &wasm_body.locals,
        wasm_body.async_state_count,
        wasm_body
            .cancellation_region
            .expect("suspending functions have a cancellation region"),
        layout,
        runtime,
        frame,
        Some(instance),
        BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        None,
        function_index,
    )
}

pub(super) fn compile_async_closure_poll(
    instance: &crate::semantic::ClosureInstance,
    closure: &wasm_ir::ClosureBody,
    function_index: u32,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.closure_frame_index(instance),
        source: AsyncFrameSource::Local(0),
    };
    compile_async_body(
        &closure.entry,
        &closure.locals,
        closure.async_state_count,
        wasm_ir::CancellationRegion::ProcessLifetime,
        layout,
        runtime,
        frame,
        instance.owner.as_ref(),
        BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        None,
        function_index,
    )
}

pub(super) fn compile_intrinsic_future_poll(
    instance: &IntrinsicFutureInstance,
    function_index: u32,
    layout: &IntrinsicFutureLayout,
    runtime: &AttachContext<'_>,
) -> Function {
    let frame = AsyncFrameRef {
        struct_type: runtime.lowering.gc.intrinsic_frame_index(instance),
        source: AsyncFrameSource::Local(0),
    };
    let planned = wasm_ir::intrinsic_future_locals(
        instance.expression,
        runtime.lowering.wasm_ir,
        runtime.lowering.semantics,
        runtime.lowering.capabilities,
    );
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut values = HashMap::new();
    plan_wasm_locals(
        &planned,
        &mut values,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1,
            semantics: runtime.lowering.semantics,
            wasm_ir: runtime.lowering.wasm_ir,
            gc: runtime.lowering.gc,
            reachability: runtime.lowering.reachability,
            instance: instance.owner.as_ref(),
            include_values: false,
        },
    );
    let mut function = Function::new(local_types.into_iter().map(|ty| (1, ty)));
    let empty_values = HashMap::new();
    let empty_temporaries = HashMap::new();
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
        reachability: runtime.lowering.reachability,
        abi: runtime.abi,
        state: runtime.lowering.state,
        locals: LocalStorage::Hybrid {
            frame,
            wasm_values: &empty_values,
            frame_values: &empty_values,
            wasm_temporaries: &empty_temporaries,
            frame_temporaries: &empty_temporaries,
        },
        globals: runtime.lowering.globals,
        global_types: runtime.lowering.global_types,
        settings: runtime.lowering.settings,
        runtime_globals: runtime.lowering.runtime_globals,
        runtime_helpers: runtime.lowering.runtime_helpers,
        functions: runtime.lowering.functions,
        closures: runtime.lowering.closures,
        function_values: runtime.lowering.function_values,
        closure_polls: runtime.lowering.closure_polls,
        closure_environment: None,
        intrinsic_futures: runtime.lowering.intrinsic_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        array_functions: runtime.lowering.array_functions,
        set_functions: runtime.lowering.set_functions,
        records: runtime.lowering.records,
        managed_classes: runtime.lowering.managed_classes,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        signatures: runtime.lowering.signatures,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        intrinsic_capture: Some(IntrinsicCapture { frame, layout }),
        debug: runtime.lowering.debug_emission(function_index),
        function_instance: instance.owner.as_ref(),
        loop_control: None,
        bare_return: BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        materialize_none: true,
    };

    frame.emit(&mut function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: frame.struct_type,
            field_index: 0,
        })
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if call_target(runtime.lowering.wasm_ir, instance.expression).and_then(resolved_intrinsic)
        == Some(IntrinsicId::NextTick)
    {
        frame.emit(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        frame.emit(&mut function);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }

    let destination_layout = AsyncFrameLayout::for_leaf_completion(layout.completion);
    compile_suspension_poll(
        &mut function,
        SuspensionMode::Await,
        wasm_ir::SuspensionDestination::BodyResult,
        instance.expression,
        runtime.abi,
        runtime.strings,
        &destination_layout,
        &context,
    );
    mark_future_complete(&mut function, context.bare_return);
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn compile_async_body(
    entry: &wasm_ir::Block,
    locals: &[wasm_ir::Local],
    async_state_count: u32,
    cancellation_region: wasm_ir::CancellationRegion,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
    frame: AsyncFrameRef,
    function_instance: Option<&FunctionInstance>,
    bare_return: BareReturn,
    result_global: Option<u32>,
    function_index: u32,
) -> Function {
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut planned_locals = HashMap::new();
    plan_wasm_locals(
        locals,
        &mut planned_locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1,
            semantics: runtime.lowering.semantics,
            wasm_ir: runtime.lowering.wasm_ir,
            gc: runtime.lowering.gc,
            reachability: runtime.lowering.reachability,
            instance: function_instance,
            include_values: true,
        },
    );
    let mut function = Function::new(local_types.into_iter().map(|ty| (1, ty)));
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
        reachability: runtime.lowering.reachability,
        abi: runtime.abi,
        state: runtime.lowering.state,
        locals: LocalStorage::Hybrid {
            frame,
            wasm_values: &planned_locals,
            frame_values: &layout.fields,
            wasm_temporaries: &matches.temporaries,
            frame_temporaries: &layout.temporaries,
        },
        globals: runtime.lowering.globals,
        global_types: runtime.lowering.global_types,
        settings: runtime.lowering.settings,
        runtime_globals: runtime.lowering.runtime_globals,
        runtime_helpers: runtime.lowering.runtime_helpers,
        functions: runtime.lowering.functions,
        closures: runtime.lowering.closures,
        function_values: runtime.lowering.function_values,
        closure_polls: runtime.lowering.closure_polls,
        closure_environment: None,
        intrinsic_futures: runtime.lowering.intrinsic_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        array_functions: runtime.lowering.array_functions,
        set_functions: runtime.lowering.set_functions,
        records: runtime.lowering.records,
        managed_classes: runtime.lowering.managed_classes,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        signatures: runtime.lowering.signatures,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        intrinsic_capture: None,
        debug: runtime.lowering.debug_emission(function_index),
        function_instance,
        loop_control: None,
        bare_return,
        materialize_none: true,
    };

    let mut states = (0..async_state_count).map(|_| None).collect::<Vec<_>>();
    states[wasm_ir::AsyncStateId::ENTRY.index() as usize] = Some(AsyncState::Block {
        block: entry,
        loop_targets: None,
        resume_source: None,
    });
    collect_async_states(entry, &mut states, None);
    debug_assert!(states.iter().all(Option::is_some));

    function.instruction(&Instruction::Loop(BlockType::Empty));
    for (pc, state) in states.into_iter().enumerate() {
        let state = state.expect("every async state is assigned during lowering");
        frame.emit(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(pc as i32))
            .instruction(&Instruction::I32Eq);

        function.instruction(&Instruction::If(BlockType::Empty));
        if let Some(debug) = context.debug {
            match state {
                AsyncState::Block { resume_source, .. } => {
                    debug.mark_resume(&function, resume_source);
                }
                AsyncState::Poll { source, .. } => {
                    debug.mark_suspend(&function, source);
                }
                AsyncState::ForHeader { .. } => {}
            }
        }

        match state {
            AsyncState::Block {
                block,
                loop_targets,
                ..
            } => compile_async_flow(
                &mut function,
                block,
                1,
                loop_targets.map(|targets| targets.control(1)),
                result_global,
                cancellation_region,
                runtime,
                layout,
                &context,
            ),
            AsyncState::ForHeader {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterator_step,
                body,
                header_state,
                exit_state,
            } => {
                compile_for_has_next(
                    &mut function,
                    iterable_value,
                    index_value,
                    version_value,
                    iterator_step,
                    &context,
                );
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_for_bind_and_advance(
                    &mut function,
                    binding,
                    iterable_value,
                    index_value,
                    version_value,
                    &context,
                );
                compile_async_flow(
                    &mut function,
                    body,
                    2,
                    Some(
                        AsyncLoopTargets {
                            break_state: exit_state,
                            continue_state: header_state,
                            break_destination: None,
                        }
                        .control(2),
                    ),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    &context,
                );
                function.instruction(&Instruction::Else);
                set_async_state(&mut function, exit_state, frame);
                function
                    .instruction(&Instruction::Br(2))
                    .instruction(&Instruction::End);
            }
            AsyncState::Poll {
                mode,
                destination,
                value,
                resume_state,
                cancellation,
                ..
            } => {
                if call_target(context.wasm_ir, value)
                    .and_then(resolved_intrinsic)
                    .is_some()
                {
                    assert_eq!(
                        cancellation,
                        Some(cancellation_region),
                        "awaited standard-library operation must participate in its body's cancellation region"
                    );
                }
                compile_suspension_poll(
                    &mut function,
                    mode,
                    destination,
                    value,
                    runtime.abi,
                    runtime.strings,
                    layout,
                    &context,
                );
                set_async_state(&mut function, resume_state, frame);
                function.instruction(&Instruction::Br(1));
            }
        }
        mark_future_complete(&mut function, bare_return);
        function
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

fn mark_future_complete(function: &mut Function, target: BareReturn) {
    if let BareReturn::AsyncFuture { frame, .. } = target {
        frame.emit(function);
        function
            .instruction(&Instruction::I32Const(-1))
            .instruction(&Instruction::StructSet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            });
    }
}

fn intrinsic_state(context: &ExprContext<'_>, slot: usize) -> (AsyncFrameRef, u32, Type) {
    let capture = context
        .intrinsic_capture
        .expect("stateful intrinsics are polled through their own future frame");
    let (field, ty) = capture.layout.state[slot];
    (capture.frame, field, ty)
}

fn emit_intrinsic_state_get(function: &mut Function, context: &ExprContext<'_>, slot: usize) {
    let (frame, field, ty) = intrinsic_state(context, slot);
    frame.emit(function);
    emit_typed_struct_get(function, frame.struct_type, field, ty);
}

fn emit_intrinsic_state_set_local(
    function: &mut Function,
    context: &ExprContext<'_>,
    slot: usize,
    local: u32,
) {
    let (frame, field, _) = intrinsic_state(context, slot);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        });
}

fn emit_intrinsic_state_set_zero(function: &mut Function, context: &ExprContext<'_>, slot: usize) {
    let (frame, field, _) = intrinsic_state(context, slot);
    frame.emit(function);
    function
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        });
}

fn emit_signature_length_i64(function: &mut Function, signature: u32) {
    function
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I64Const(32))
        .instruction(&Instruction::I64ShrU);
}

fn emit_signature_length_i32(function: &mut Function, signature: u32) {
    emit_signature_length_i64(function, signature);
    function.instruction(&Instruction::I32WrapI64);
}

fn emit_signature_window_limit(function: &mut Function, signature: u32) {
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64Const(
            SIGNATURE_SCAN_CANDIDATES_PER_POLL - 1,
        ))
        .instruction(&Instruction::I64Add);
}

fn emit_signature_arguments(function: &mut Function, signature: u32) {
    // A first-class Signature packs its static needle pointer into the low 32
    // bits and its length into the high 32 bits. The mask immediately follows
    // the needle in the compiler-owned static-data pool.
    function
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalGet(signature))
        .instruction(&Instruction::I32WrapI64);
    emit_signature_length_i32(function, signature);
    function.instruction(&Instruction::I32Add);
    emit_signature_length_i32(function, signature);
}

/// Polls one bounded window of an explicit memory range. The caller has
/// already placed the range address and size in scratch slots 1 and 2. A
/// successful address is left in slot 3; absence returns `pending` directly.
fn emit_cooperative_range_scan(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    scratch: &[u32],
    process_from_runtime: bool,
    relative_target: Option<(u32, u32)>,
    context: &ExprContext<'_>,
) {
    let cursor = scratch[0];
    let address = scratch[1];
    let remaining = scratch[2];
    let matched = scratch[3];
    let signature = scratch[4];

    // Deliver a match saved by the previous poll without doing more scanning
    // work. A scan that inspects memory always yields once, so several awaited
    // scans chained by the caller cannot all run during a single host update.
    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::LocalSet(cursor))
        // remaining = size - cursor, unless no candidate can start here.
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if process_from_runtime {
        function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    } else {
        compile_receiver(function, target, context);
    }
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    if let Some((displacement_offset, relative_target)) = relative_target {
        function
            .instruction(&Instruction::LocalGet(displacement_offset))
            .instruction(&Instruction::LocalGet(relative_target));
    }
    function
        .instruction(&Instruction::Call(context.runtime_helpers.function(
            if relative_target.is_some() {
                RuntimeHelperId::ScanRelative32TargetRange
            } else {
                RuntimeHelperId::ScanProcessRange
            },
        )))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    function.instruction(&Instruction::Else);
    let (frame, field, _) = intrinsic_state(context, 0);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        })
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 1, matched);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
}

fn emit_process_scan_advance_range(
    function: &mut Function,
    context: &ExprContext<'_>,
    next_range: u32,
) {
    emit_intrinsic_state_set_local(function, context, 0, next_range);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

/// Polls at most one bounded window from one readable process memory range.
/// State slot 0 encodes the current reverse range as `index + 1`; zero starts
/// a fresh snapshot. State slot 1 is the byte cursor within that range.
fn emit_cooperative_process_scan(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let range_cursor = scratch[0];
    let offset = scratch[1];
    let index = scratch[2];
    let address = scratch[3];
    let remaining = scratch[4];
    let matched = scratch[5];
    let signature = scratch[6];

    // A non-zero result was found in the preceding poll. Deliver it now and
    // clear the future-local traversal state for a later execution of the same
    // await expression.
    emit_intrinsic_state_get(function, context, 2);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(range_cursor));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(offset));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(index))
        // Start a new reverse traversal, or recover if the host's mapping list
        // shrank while this future was suspended.
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(range_cursor))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(index));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::I64Const(1 << 1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_advance_range(function, context, index);
    function.instruction(&Instruction::Else);
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    let (frame, field, _) = intrinsic_state(context, 1);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: field,
        })
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 2, matched);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
}

fn emit_process_scan_any_advance_range(
    function: &mut Function,
    context: &ExprContext<'_>,
    next_range: u32,
) {
    emit_intrinsic_state_set_local(function, context, 0, next_range);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return);
}

fn emit_signature_array_length(
    function: &mut Function,
    signatures_expression: ExprId,
    context: &ExprContext<'_>,
) {
    compile_expr(function, signatures_expression, context);
    let Type::Array(array) = context.expression_type(signatures_expression) else {
        unreachable!("signature candidates are arrays")
    };
    super::array_value::emit_length(function, context.gc, array);
    function.instruction(&Instruction::I64ExtendI32U);
}

fn emit_cooperative_module_scan_any(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    signatures_expression: ExprId,
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    context: &ExprContext<'_>,
) {
    let offset = scratch[0];
    let remaining = scratch[1];
    let matched = scratch[2];
    let signature = scratch[3];
    let signature_index = scratch[4];
    let Type::Array(signature_array) = context.expression_type(signatures_expression) else {
        unreachable!("module scan candidates are arrays")
    };

    emit_intrinsic_state_get(function, context, 2);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(signature_index));
    compile_expr(function, signatures_expression, context);
    super::array_value::emit_backing(function, context.gc, signature_array);
    let signature_storage =
        super::array_value::storage_id(signature_array, context.arrays, context.semantics);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::ArrayGet(
            context.gc.index(Type::ArrayStorage(signature_storage)),
        ))
        .instruction(&Instruction::LocalSet(signature));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
            field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
        })
        .instruction(&Instruction::LocalTee(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
            field_index: context
                .gc
                .standard_field_index(StdlibFieldId::ModuleAddress),
        })
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(signature_index));
    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 1, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_set_local(function, context, 0, offset);
    emit_intrinsic_state_set_zero(function, context, 1);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_set_local(function, context, 2, matched);
    emit_intrinsic_state_set_local(function, context, 3, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if let Some((field, _)) = layout.field(destination) {
        context.locals.frame().emit(function);
        function.instruction(&Instruction::LocalGet(matched));
        emit_intrinsic_state_get(function, context, 3);
        function
            .instruction(&Instruction::I32WrapI64)
            .instruction(&Instruction::StructNew(
                context.gc.standard_index(StdlibTypeId::SignatureMatch),
            ))
            .instruction(&Instruction::StructSet {
                struct_type_index: context.locals.frame().struct_type,
                field_index: field,
            });
    }
    for slot in 0..4 {
        emit_intrinsic_state_set_zero(function, context, slot);
    }
}

/// Traverses at most one mapped range per poll and completes after one
/// snapshot of the host's range list. Status values are zero (uninitialized),
/// one (found), two (exhausted), and three (scanning).
#[allow(clippy::too_many_arguments)]
fn emit_process_find_memory_range(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    args: &[ExprId],
    destination: wasm_ir::SuspensionDestination,
    layout: &AsyncFrameLayout,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let status = scratch[0];
    let cursor = scratch[1];
    let address = scratch[2];
    let size = scratch[3];
    let flags = scratch[4];

    emit_intrinsic_state_get(function, context, 0);
    function
        .instruction(&Instruction::LocalSet(status))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    if let Some((field, Type::Option(option))) = layout.field(destination) {
        context.locals.frame().emit(function);
        function
            .instruction(&Instruction::LocalGet(status))
            .instruction(&Instruction::I64Const(1))
            .instruction(&Instruction::I64Eq)
            .instruction(&Instruction::If(BlockType::Result(
                context.gc.val_type(Type::Option(option)),
            )));
        emit_intrinsic_state_get(function, context, 2);
        emit_intrinsic_state_get(function, context, 3);
        for mask in [2_i64, 4, 8] {
            emit_intrinsic_state_get(function, context, 4);
            function
                .instruction(&Instruction::I64Const(mask))
                .instruction(&Instruction::I64And)
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::I32Eqz);
        }
        function
            .instruction(&Instruction::StructNew(
                context.gc.standard_index(StdlibTypeId::MemoryRange),
            ))
            .instruction(&Instruction::StructNew(
                context.gc.index(Type::Option(option)),
            ))
            .instruction(&Instruction::Else)
            .instruction(&Instruction::RefNull(HeapType::Concrete(
                context.gc.index(Type::Option(option)),
            )))
            .instruction(&Instruction::End)
            .instruction(&Instruction::StructSet {
                struct_type_index: context.locals.frame().struct_type,
                field_index: field,
            });
    }
    for slot in 0..5 {
        emit_intrinsic_state_set_zero(function, context, slot);
    }
    function
        .instruction(&Instruction::Br(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(cursor));
    emit_intrinsic_state_set_local(function, context, 1, cursor);
    function
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::LocalSet(status));
    emit_intrinsic_state_set_local(function, context, 0, status);
    function.instruction(&Instruction::End);

    emit_intrinsic_state_get(function, context, 1);
    function
        .instruction(&Instruction::LocalTee(cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::LocalSet(status));
    emit_intrinsic_state_set_local(function, context, 0, status);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(cursor));
    emit_intrinsic_state_set_local(function, context, 1, cursor);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(size));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::LocalSet(flags))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(size));
    compile_expr(function, args[0], context);
    function
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(flags));
    compile_expr(function, args[1], context);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::MemoryRangeAccess),
            field_index: 0,
        })
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::Else);
    compile_expr(function, args[1], context);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: context.gc.standard_index(StdlibTypeId::MemoryRangeAccess),
            field_index: 0,
        })
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(6))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(status))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 2, address);
    emit_intrinsic_state_set_local(function, context, 3, size);
    emit_intrinsic_state_set_local(function, context, 4, flags);
    function
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::LocalSet(status));
    emit_intrinsic_state_set_local(function, context, 0, status);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

/// Multi-pattern counterpart of `emit_cooperative_process_scan`. One poll
/// checks one signature in one bounded window. This keeps latency independent
/// of both mapped-range size and the number of fallback signatures.
fn emit_cooperative_process_scan_any(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    signatures_expression: ExprId,
    signature_array: crate::ast::ArrayTypeId,
    scratch: &[u32],
    abi: &Abi,
    context: &ExprContext<'_>,
) {
    let range_cursor = scratch[0];
    let offset = scratch[1];
    let index = scratch[2];
    let address = scratch[3];
    let remaining = scratch[4];
    let matched = scratch[5];
    let signature = scratch[6];
    let signature_index = scratch[7];

    // Preserve both parts of the match across one mandatory cooperative
    // boundary. This keeps a fallback scan to one signature/window attempt per
    // host poll even when its caller immediately awaits another scan.
    emit_intrinsic_state_get(function, context, 3);
    function
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));

    emit_intrinsic_state_get(function, context, 0);
    function.instruction(&Instruction::LocalSet(range_cursor));
    emit_intrinsic_state_get(function, context, 1);
    function.instruction(&Instruction::LocalSet(offset));
    emit_intrinsic_state_get(function, context, 2);
    function.instruction(&Instruction::LocalSet(signature_index));

    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    compile_expr(function, signatures_expression, context);
    super::array_value::emit_backing(function, context.gc, signature_array);
    let signature_storage =
        super::array_value::storage_id(signature_array, context.arrays, context.semantics);
    function
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::ArrayGet(
            context.gc.index(Type::ArrayStorage(signature_storage)),
        ))
        .instruction(&Instruction::LocalSet(signature));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(range_cursor))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(signature_index))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(range_cursor))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalSet(index));

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::I64Const(1 << 1))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalSet(address));
    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining));
    emit_signature_length_i64(function, signature);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::End);

    compile_receiver(function, target, context);
    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::Else);
    emit_signature_window_limit(function, signature);
    function.instruction(&Instruction::End);
    emit_signature_arguments(function, signature);
    function
        .instruction(&Instruction::Call(
            context
                .runtime_helpers
                .function(RuntimeHelperId::ScanProcessRange),
        ))
        .instruction(&Instruction::LocalTee(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        // Try the next signature against this same range window.
        .instruction(&Instruction::LocalGet(signature_index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalTee(signature_index));
    emit_signature_array_length(function, signatures_expression, context);
    function
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    emit_intrinsic_state_set_local(function, context, 1, offset);
    emit_intrinsic_state_set_local(function, context, 2, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // Every signature missed this window; move to the next window/range.
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(signature_index))
        .instruction(&Instruction::LocalGet(remaining));
    emit_signature_window_limit(function, signature);
    function
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_scan_any_advance_range(function, context, index);
    function.instruction(&Instruction::Else);
    emit_intrinsic_state_set_local(function, context, 0, range_cursor);
    let (frame, offset_field, _) = intrinsic_state(context, 1);
    frame.emit(function);
    function
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(SIGNATURE_SCAN_CANDIDATES_PER_POLL))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: offset_field,
        });
    emit_intrinsic_state_set_zero(function, context, 2);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    emit_intrinsic_state_set_local(function, context, 3, matched);
    emit_intrinsic_state_set_local(function, context, 4, signature_index);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
    emit_intrinsic_state_get(function, context, 4);
    function.instruction(&Instruction::LocalSet(signature_index));
    emit_intrinsic_state_set_zero(function, context, 0);
    emit_intrinsic_state_set_zero(function, context, 1);
    emit_intrinsic_state_set_zero(function, context, 2);
    emit_intrinsic_state_set_zero(function, context, 3);
    emit_intrinsic_state_set_zero(function, context, 4);
}

#[allow(clippy::too_many_arguments)]
fn compile_suspension_poll(
    function: &mut Function,
    mode: SuspensionMode,
    destination: wasm_ir::SuspensionDestination,
    value: ExprId,
    abi: &Abi,
    strings: &StringPool,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    if mode == SuspensionMode::Retry {
        compile_retry_poll(function, destination, value, layout, context);
        return;
    }
    let value_expression = context
        .wasm_ir
        .expression(value)
        .expect("await value belongs to Wasm IR");
    let stateful_intrinsic = match &value_expression.kind {
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::Intrinsic { intrinsic, .. },
            ..
        } => crate::intrinsic_registry::contract(*intrinsic)
            .async_state
            .is_some(),
        _ => false,
    };
    if !matches!(
        &value_expression.kind,
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::Intrinsic { .. },
            ..
        }
    ) || (stateful_intrinsic && context.intrinsic_capture.is_none())
    {
        compile_source_future_poll(function, destination, value, layout, context);
        return;
    }
    let wasm_ir::ExpressionKind::Call {
        target,
        arguments: args,
    } = &value_expression.kind
    else {
        unreachable!();
    };
    let scratch = context
        .matches
        .intrinsic_temps
        .get(&value)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let primary_scratch = scratch.first().copied().unwrap_or(u32::MAX);
    let module_address_local = primary_scratch;
    let module_size_local = scratch.get(1).copied().unwrap_or(u32::MAX);
    let unity_image_local = primary_scratch;
    let unity_class_local = primary_scratch;
    let unity_field_local = primary_scratch;
    match resolved_intrinsic(target) {
        Some(IntrinsicId::NextTick) => {}
        Some(IntrinsicId::ProcessClosed) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::Drop)
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return);
        }
        Some(IntrinsicId::ProcessMainModule | IntrinsicId::ProcessModule) => {
            let module_name = if resolved_intrinsic(target) == Some(IntrinsicId::ProcessModule) {
                let wasm_ir::ExpressionKind::String(name) = &context
                    .wasm_ir
                    .expression(args[0])
                    .expect("module name belongs to Wasm IR")
                    .kind
                else {
                    unreachable!();
                };
                Some(name.as_str())
            } else {
                None
            };
            emit_process_module_query(
                function,
                target,
                module_name,
                AbiImportId::ProcessGetModuleAddress,
                abi,
                strings,
                context,
            );
            function
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            emit_process_module_query(
                function,
                target,
                module_name,
                AbiImportId::ProcessGetModuleSize,
                abi,
                strings,
                context,
            );
            function
                .instruction(&Instruction::LocalTee(module_size_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::LocalGet(module_size_local));
                emit_module_name_value(function, module_name, context);
                function
                    .instruction(&Instruction::StructNew(
                        context.gc.standard_index(StdlibTypeId::Module),
                    ))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessFindMemoryRange) => {
            emit_process_find_memory_range(
                function,
                target,
                args,
                destination,
                layout,
                scratch,
                abi,
                context,
            );
        }
        Some(IntrinsicId::ProcessRead) => {
            let read_type_id = match target {
                wasm_ir::CallTarget::Intrinsic { type_arguments, .. } => {
                    context.type_id(type_arguments[0])
                }
                _ => unreachable!("process.read must resolve to its standard-library item"),
            };
            let read_type = semantic_type(read_type_id, context.semantics);
            let read_size = context
                .memory
                .layout(read_type_id, context.semantics)
                .expect("checked process reads are MemoryReadable")
                .size();
            if let Some((_, stored_type)) = layout.field(destination) {
                context.locals.frame().emit(function);
                debug_assert_eq!(stored_type, read_type);
            }
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::I32Const(
                    context.abi_read.destination(read_size),
                ))
                .instruction(&Instruction::I32Const(read_size as i32))
                .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if read_type == Type::Address {
                function
                    .instruction(&Instruction::I32Const(context.abi_read.start()))
                    .instruction(&Instruction::I64Load(memarg()))
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            if let Some((field, _)) = layout.field(destination) {
                emit_memory_value(
                    function,
                    read_type_id,
                    context.abi_read,
                    0,
                    context.memory,
                    context.semantics,
                    context.gc,
                    MemoryByteOrder::Little,
                );
                function.instruction(&Instruction::StructSet {
                    struct_type_index: context.locals.frame().struct_type,
                    field_index: field,
                });
            }
        }
        Some(IntrinsicId::ProcessFollow) => {
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            compile_expr(function, args[1], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::FollowAddress),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScan) => {
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[1]));
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[2], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            emit_cooperative_range_scan(function, target, scratch, false, None, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleScanRelative32Target) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::ModuleAddress),
                })
                .instruction(&Instruction::LocalSet(scratch[1]));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
                })
                .instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            compile_expr(function, args[1], context);
            function.instruction(&Instruction::LocalSet(scratch[5]));
            compile_expr(function, args[2], context);
            function.instruction(&Instruction::LocalSet(scratch[6]));
            emit_cooperative_range_scan(
                function,
                target,
                scratch,
                true,
                Some((scratch[5], scratch[6])),
                context,
            );
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScanMemory) => {
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[6]));
            emit_cooperative_process_scan(function, target, scratch, abi, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[5]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessScanMemoryAny) => {
            let Type::Array(signature_array) = context.expression_type(args[0]) else {
                unreachable!("scanMemoryAny accepts a signature array")
            };
            emit_cooperative_process_scan_any(
                function,
                target,
                args[0],
                signature_array,
                scratch,
                abi,
                context,
            );
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[5]))
                    .instruction(&Instruction::LocalGet(scratch[7]))
                    .instruction(&Instruction::I32WrapI64)
                    .instruction(&Instruction::StructNew(
                        context.gc.standard_index(StdlibTypeId::SignatureMatch),
                    ))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessReadRelative32) => {
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ReadRelative32),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityModuleImage) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityGetImage),
                ))
                .instruction(&Instruction::LocalTee(unity_image_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(unity_image_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityImageClass) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityGetClass),
                ))
                .instruction(&Instruction::LocalTee(unity_class_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(unity_class_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassFieldAny) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityGetFieldAny),
                ))
                .instruction(&Instruction::LocalTee(unity_field_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(unity_field_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassField) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityGetFieldOffset),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::I64Const(1))
                    .instruction(&Instruction::I64Sub)
                    .instruction(&Instruction::I32WrapI64)
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassStaticInstance) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityGetStaticInstance),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassStaticTable) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::UnityClass),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::UnityClassModule),
                })
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::UnityModule),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::UnityModuleVersion),
                })
                .instruction(&Instruction::I64ExtendI32U)
                .instruction(&Instruction::LocalSet(module_address_local))
                .instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::UnityClass),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::UnityClassAddress),
                });
            unity_layout::emit_versioned_offset(
                function,
                module_address_local,
                unity_layout::VersionedOffset::ClassStaticTable,
            );
            function
                .instruction(&Instruction::I64Add)
                .instruction(&Instruction::I32Const(
                    context.abi_read.destination(unity_layout::POINTER_SIZE),
                ))
                .instruction(&Instruction::I32Const(unity_layout::POINTER_SIZE as i32))
                .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End)
                .instruction(&Instruction::I32Const(context.abi_read.start()))
                .instruction(&Instruction::I64Load(memarg()))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleScan) => {
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::ModuleAddress),
                })
                .instruction(&Instruction::LocalSet(scratch[1]));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
                })
                .instruction(&Instruction::LocalSet(scratch[2]));
            compile_expr(function, args[0], context);
            function.instruction(&Instruction::LocalSet(scratch[4]));
            emit_cooperative_range_scan(function, target, scratch, true, None, context);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(scratch[3]))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: context.locals.frame().struct_type,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ModuleScanAny) => {
            emit_cooperative_module_scan_any(
                function,
                target,
                args[0],
                destination,
                layout,
                scratch,
                context,
            );
        }
        _ => unreachable!("type checking only permits awaitable builtins"),
    }
}

fn compile_source_future_poll(
    function: &mut Function,
    destination: wasm_ir::SuspensionDestination,
    expression: ExprId,
    parent_layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    let (child_field, child_type) = parent_layout.children[&expression];
    let Type::Async(child_future) = child_type else {
        unreachable!("source async calls produce future values")
    };
    let parent = context.locals.frame();

    parent.emit(function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: parent.struct_type,
            field_index: child_field,
        })
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty));
    parent.emit(function);
    compile_expr(function, expression, context);
    function
        .instruction(&Instruction::StructSet {
            struct_type_index: parent.struct_type,
            field_index: child_field,
        })
        .instruction(&Instruction::End);

    let source_candidates = context
        .async_frames
        .functions()
        .filter(|(candidate, _)| {
            let result = context.semantics.specialize_type(
                candidate,
                context
                    .semantics
                    .function_result(candidate.function)
                    .expect("checked functions have result types"),
            );
            semantic_type(result, context.semantics) == child_type
        })
        .collect::<Vec<_>>();
    let closure_candidates = context
        .async_frames
        .closures()
        .filter(|(instance, _)| {
            let ty = context
                .wasm_ir
                .expression(instance.expression)
                .expect("reachable closure expressions belong to Wasm IR")
                .ty;
            let ty = instance
                .owner
                .as_ref()
                .map_or(ty, |owner| context.semantics.specialize_type(owner, ty));
            let TypeKind::Callable { result, .. } = context.semantics.types().kind(ty) else {
                unreachable!("checked closure expressions have callable types")
            };
            semantic_type(*result, context.semantics) == child_type
        })
        .collect::<Vec<_>>();
    let intrinsic_candidates = context
        .async_frames
        .intrinsics()
        .filter(|(_, layout)| layout.future == child_type)
        .collect::<Vec<_>>();
    assert!(
        !source_candidates.is_empty()
            || !closure_candidates.is_empty()
            || !intrinsic_candidates.is_empty(),
        "reachable future values have at least one concrete producer"
    );
    function.instruction(&Instruction::Block(BlockType::Empty));
    for (child, child_layout) in source_candidates {
        let child_frame = context.gc.function_frame_index(child);
        parent.emit(function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: parent.struct_type,
                field_index: child_field,
            })
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: context.gc.index(Type::Async(child_future)),
                field_index: 1,
            })
            .instruction(&Instruction::I32Const(
                context.gc.function_frame_tag(child) as i32
            ))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_child_frame(function, parent, child_field, child_frame);
        function
            .instruction(&Instruction::Call(
                context.functions[child]
                    .poll
                    .expect("async source functions have poll entries"),
            ))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);

        if let Some((destination_field, destination_type)) = parent_layout.field(destination) {
            parent.emit(function);
            if let Some((completion_field, completion_type)) = child_layout.completion {
                debug_assert_eq!(destination_type, completion_type);
                emit_child_frame(function, parent, child_field, child_frame);
                emit_typed_struct_get(function, child_frame, completion_field, completion_type);
            } else {
                debug_assert_eq!(destination_type, Type::None);
                emit_default(function, Type::None, context.gc);
            }
            function.instruction(&Instruction::StructSet {
                struct_type_index: parent.struct_type,
                field_index: destination_field,
            });
        }

        clear_child_future(function, parent, child_field, child_future, context);
        function
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }
    for (child, child_layout) in closure_candidates {
        let child_frame = context.gc.closure_frame_index(child);
        parent.emit(function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: parent.struct_type,
                field_index: child_field,
            })
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: context.gc.index(Type::Async(child_future)),
                field_index: 1,
            })
            .instruction(&Instruction::I32Const(
                context.gc.closure_frame_tag(child) as i32
            ))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_child_frame(function, parent, child_field, child_frame);
        function
            .instruction(&Instruction::Call(context.closure_polls[&child]))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);

        if let Some((destination_field, destination_type)) = parent_layout.field(destination) {
            parent.emit(function);
            if let Some((completion_field, completion_type)) = child_layout.completion {
                debug_assert_eq!(destination_type, completion_type);
                emit_child_frame(function, parent, child_field, child_frame);
                emit_typed_struct_get(function, child_frame, completion_field, completion_type);
            } else {
                debug_assert_eq!(destination_type, Type::None);
                emit_default(function, Type::None, context.gc);
            }
            function.instruction(&Instruction::StructSet {
                struct_type_index: parent.struct_type,
                field_index: destination_field,
            });
        }

        clear_child_future(function, parent, child_field, child_future, context);
        function
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }
    for (child, child_layout) in intrinsic_candidates {
        let child_frame = context.gc.intrinsic_frame_index(child);
        parent.emit(function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: parent.struct_type,
                field_index: child_field,
            })
            .instruction(&Instruction::RefAsNonNull)
            .instruction(&Instruction::StructGet {
                struct_type_index: context.gc.index(Type::Async(child_future)),
                field_index: 1,
            })
            .instruction(&Instruction::I32Const(
                context.gc.intrinsic_frame_tag(child) as i32,
            ))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_child_frame(function, parent, child_field, child_frame);
        function
            .instruction(&Instruction::Call(context.intrinsic_futures[child]))
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);

        if let Some((destination_field, destination_type)) = parent_layout.field(destination) {
            parent.emit(function);
            if let Some((completion_field, completion_type)) = child_layout.completion {
                debug_assert_eq!(destination_type, completion_type);
                emit_child_frame(function, parent, child_field, child_frame);
                emit_typed_struct_get(function, child_frame, completion_field, completion_type);
            } else {
                debug_assert_eq!(destination_type, Type::None);
                emit_default(function, Type::None, context.gc);
            }
            function.instruction(&Instruction::StructSet {
                struct_type_index: parent.struct_type,
                field_index: destination_field,
            });
        }

        clear_child_future(function, parent, child_field, child_future, context);
        function
            .instruction(&Instruction::Br(1))
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
}

fn clear_child_future(
    function: &mut Function,
    parent: AsyncFrameRef,
    field: u32,
    future: crate::ast::AsyncTypeId,
    context: &ExprContext<'_>,
) {
    parent.emit(function);
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            context.gc.index(Type::Async(future)),
        )))
        .instruction(&Instruction::StructSet {
            struct_type_index: parent.struct_type,
            field_index: field,
        });
}

fn emit_child_frame(function: &mut Function, parent: AsyncFrameRef, field: u32, child_frame: u32) {
    parent.emit(function);
    function
        .instruction(&Instruction::StructGet {
            struct_type_index: parent.struct_type,
            field_index: field,
        })
        .instruction(&Instruction::RefCastNonNull(HeapType::Concrete(
            child_frame,
        )));
}

fn emit_module_name_value(
    function: &mut Function,
    module_name: Option<&str>,
    context: &ExprContext<'_>,
) {
    if let Some(name) = module_name {
        emit_string_literal(function, name, context.gc);
        return;
    }

    let names = provider_process_names(context);
    debug_assert!(!names.is_empty());
    let string_type = context.gc.val_type(Type::Standard(StdlibTypeId::String));
    for (index, name) in names.iter().enumerate() {
        function
            .instruction(&Instruction::GlobalGet(
                context.runtime_globals.process_name,
            ))
            .instruction(&Instruction::I32Const(index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(string_type)));
        emit_string_literal(function, name, context.gc);
        function.instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::Unreachable);
    for _ in names {
        function.instruction(&Instruction::End);
    }
}

fn emit_process_module_query(
    function: &mut Function,
    target: &wasm_ir::CallTarget,
    module_name: Option<&str>,
    import: AbiImportId,
    abi: &Abi,
    strings: &StringPool,
    context: &ExprContext<'_>,
) {
    if let Some(name) = module_name {
        let (ptr, len) = strings.get(name);
        compile_receiver(function, target, context);
        function
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(abi.function(import)));
        return;
    }

    let names = provider_process_names(context);
    debug_assert!(!names.is_empty());
    for (index, name) in names.iter().enumerate() {
        function
            .instruction(&Instruction::GlobalGet(
                context.runtime_globals.process_name,
            ))
            .instruction(&Instruction::I32Const(index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        let (ptr, len) = strings.get(name);
        compile_receiver(function, target, context);
        function
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(abi.function(import)))
            .instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::Unreachable);
    for _ in names {
        function.instruction(&Instruction::End);
    }
}

fn provider_process_names<'a>(context: &'a ExprContext<'_>) -> Vec<&'a str> {
    let provider = context
        .semantics
        .state_provider()
        .map(|provider| context.standard_library.state_provider(provider))
        .expect("checked states resolve a process provider");
    match provider.processes {
        crate::stdlib::StateProviderProcesses::Declared(processes) => processes.to_vec(),
        crate::stdlib::StateProviderProcesses::SourceState => {
            context.state.processes.iter().map(String::as_str).collect()
        }
    }
}

fn compile_retry_poll(
    function: &mut Function,
    destination: wasm_ir::SuspensionDestination,
    expression: ExprId,
    frame: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    let expression_type = context
        .wasm_ir
        .expression(expression)
        .expect("retried expression belongs to Wasm IR")
        .ty;
    let TypeKind::Result {
        layout: result,
        value: result_value,
    } = context.semantics.types().kind(expression_type)
    else {
        unreachable!("type checking only permits retrying Result expressions")
    };
    let result_local = context.matches.suspension_temps[&expression];

    compile_expr(function, expression, context);
    function
        .instruction(&Instruction::LocalTee(result_local))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        function,
        context.gc.index(Type::Result(*result)),
        1,
        Type::I32,
    );
    function
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    if let Some((field, stored_type)) = frame.field(destination) {
        debug_assert_eq!(stored_type, semantic_type(*result_value, context.semantics));
        context.locals.frame().emit(function);
        function
            .instruction(&Instruction::LocalGet(result_local))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            function,
            context.gc.index(Type::Result(*result)),
            0,
            stored_type,
        );
        function.instruction(&Instruction::StructSet {
            struct_type_index: context.locals.frame().struct_type,
            field_index: field,
        });
    }
}

#[derive(Clone, Copy)]
enum AsyncState<'a> {
    Block {
        block: &'a wasm_ir::Block,
        loop_targets: Option<AsyncLoopTargets>,
        resume_source: Option<crate::ast::Span>,
    },
    ForHeader {
        binding: ValueId,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterator_step: Option<ExprId>,
        body: &'a wasm_ir::Block,
        header_state: wasm_ir::AsyncStateId,
        exit_state: wasm_ir::AsyncStateId,
    },
    Poll {
        mode: SuspensionMode,
        destination: wasm_ir::SuspensionDestination,
        value: ExprId,
        resume_state: wasm_ir::AsyncStateId,
        cancellation: Option<wasm_ir::CancellationRegion>,
        source: Option<crate::ast::Span>,
    },
}

#[derive(Clone, Copy)]
struct AsyncLoopTargets {
    break_state: wasm_ir::AsyncStateId,
    continue_state: wasm_ir::AsyncStateId,
    break_destination: Option<wasm_ir::TemporaryId>,
}

impl AsyncLoopTargets {
    fn control(self, dispatcher_depth: u32) -> LoopControl {
        LoopControl::Async {
            break_state: self.break_state,
            continue_state: self.continue_state,
            dispatcher_depth,
            break_destination: self.break_destination,
        }
    }
}

fn collect_async_states<'a>(
    block: &'a wasm_ir::Block,
    states: &mut [Option<AsyncState<'a>>],
    loop_targets: Option<AsyncLoopTargets>,
) {
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::If {
                then_block,
                else_block,
                ..
            } => {
                collect_async_states(then_block, states, loop_targets);
                collect_async_states(else_block, states, loop_targets);
            }
            wasm_ir::Statement::Match { arms, .. } => {
                for arm in arms {
                    collect_async_states(&arm.block, states, loop_targets);
                }
            }
            wasm_ir::Statement::Fallback {
                fallback_block,
                success_block,
                ..
            } => {
                collect_async_states(fallback_block, states, loop_targets);
                collect_async_states(success_block, states, loop_targets);
            }
            wasm_ir::Statement::While { body, .. } => {
                collect_async_states(body, states, loop_targets)
            }
            wasm_ir::Statement::For { body, .. } => {
                collect_async_states(body, states, loop_targets)
            }
            wasm_ir::Statement::Store { .. }
            | wasm_ir::Statement::StateStore { .. }
            | wasm_ir::Statement::DebugLocation(_)
            | wasm_ir::Statement::StoreTemporary { .. }
            | wasm_ir::Statement::IndexStore { .. }
            | wasm_ir::Statement::Evaluate { .. }
            | wasm_ir::Statement::ForInit { .. } => {}
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Suspend {
            mode,
            destination,
            value,
            poll_state,
            resume_state,
            cancellation,
            source,
            continuation,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Poll {
                mode: *mode,
                destination: *destination,
                value: *value,
                resume_state: *resume_state,
                cancellation: *cancellation,
                source: *source,
            });
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: *source,
            });
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Retry {
            attempt,
            continuation,
            source,
            poll_state,
            resume_state,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Block {
                block: attempt,
                loop_targets,
                resume_source: *source,
            });
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: *source,
            });
            collect_async_states(attempt, states, loop_targets);
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncWhile {
            header,
            continuation,
            header_state,
            exit_state,
            result,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
                break_destination: *result,
            };
            states[header_state.index() as usize] = Some(AsyncState::Block {
                block: header,
                loop_targets: Some(inner_targets),
                resume_source: None,
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: None,
            });
            collect_async_states(header, states, Some(inner_targets));
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncWhileCondition { body, .. } => {
            collect_async_states(body, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncFor {
            binding,
            iterable_value,
            index_value,
            version_value,
            iterator_step,
            body,
            continuation,
            header_state,
            exit_state,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
                break_destination: None,
            };
            states[header_state.index() as usize] = Some(AsyncState::ForHeader {
                binding: *binding,
                iterable_value: *iterable_value,
                index_value: *index_value,
                version_value: *version_value,
                iterator_step: *iterator_step,
                body,
                header_state: *header_state,
                exit_state: *exit_state,
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
                resume_source: None,
            });
            collect_async_states(body, states, Some(inner_targets));
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Fallthrough
        | wasm_ir::Terminator::Break(_)
        | wasm_ir::Terminator::Continue
        | wasm_ir::Terminator::Return(_)
        | wasm_ir::Terminator::RetryComplete { .. }
        | wasm_ir::Terminator::Throw { .. } => {}
    }
}

fn set_async_state(function: &mut Function, state: wasm_ir::AsyncStateId, frame: AsyncFrameRef) {
    frame.emit(function);
    function
        .instruction(&Instruction::I32Const(state.index() as i32))
        .instruction(&Instruction::StructSet {
            struct_type_index: frame.struct_type,
            field_index: 0,
        });
}

#[allow(clippy::too_many_arguments)]
fn compile_async_flow(
    function: &mut Function,
    block: &wasm_ir::Block,
    loop_depth: u32,
    loop_control: Option<LoopControl>,
    result_global: Option<u32>,
    cancellation_region: wasm_ir::CancellationRegion,
    runtime: &AttachContext<'_>,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
) {
    let expression_context = ExprContext {
        loop_control,
        ..*context
    };
    let context = &expression_context;
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::DebugLocation(source) => {
                if let Some(debug) = context.debug {
                    debug.mark(function, Some(*source));
                }
            }
            wasm_ir::Statement::Store {
                target,
                declaration,
                operation,
                value,
            } => {
                compile_assignment(
                    function,
                    *target,
                    *declaration,
                    operation.as_ref(),
                    *value,
                    context,
                );
            }
            wasm_ir::Statement::StateStore {
                target,
                operation,
                value,
            } => super::expression::compile_state_assignment(
                function,
                *target,
                operation.as_ref(),
                *value,
                context,
            ),
            wasm_ir::Statement::StoreTemporary { target, value } => {
                compile_temporary_set(function, *target, *value, context);
            }
            wasm_ir::Statement::IndexStore {
                target,
                operation,
                value,
            } => super::expression::compile_index_assignment(
                function, *target, operation, *value, context,
            ),
            wasm_ir::Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                compile_expr(function, *condition, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_async_flow(
                    function,
                    then_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function.instruction(&Instruction::Else);
                compile_async_flow(
                    function,
                    else_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function.instruction(&Instruction::End);
            }
            wasm_ir::Statement::Match {
                expression,
                value,
                arms,
            } => {
                let value_local = context.matches.values[expression];
                let value_type = context.expression_type(*value);
                compile_expr(function, *value, context);
                function.instruction(&Instruction::LocalSet(value_local));
                for (arm_index, arm) in arms.iter().enumerate() {
                    let binding = compile_statement_pattern(
                        function,
                        &arm.pattern,
                        value_local,
                        value_type,
                        context,
                    );
                    let arm_context = ExprContext {
                        loop_control: loop_control
                            .map(|control| control.nested(arm_index as u32 + 1)),
                        ..*context
                    };
                    if binding.is_some() || arm.guard.is_some() {
                        function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
                        if let Some(binding) = binding {
                            store_match_binding(function, binding, value_local, &arm_context);
                        }
                        if let Some(guard) = arm.guard {
                            compile_expr(function, guard, &arm_context);
                        } else {
                            function.instruction(&Instruction::I32Const(1));
                        }
                        function
                            .instruction(&Instruction::Else)
                            .instruction(&Instruction::I32Const(0))
                            .instruction(&Instruction::End);
                    }
                    function.instruction(&Instruction::If(BlockType::Empty));
                    compile_async_flow(
                        function,
                        &arm.block,
                        loop_depth + arm_index as u32 + 1,
                        arm_context.loop_control,
                        result_global,
                        cancellation_region,
                        runtime,
                        layout,
                        &arm_context,
                    );
                    function.instruction(&Instruction::Else);
                }
                function.instruction(&Instruction::Unreachable);
                for _ in arms {
                    function.instruction(&Instruction::End);
                }
            }
            wasm_ir::Statement::Fallback {
                expression,
                value,
                fallback_block,
                success_block,
            } => {
                compile_fallback_condition(function, *expression, *value, context);
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_async_flow(
                    function,
                    fallback_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function.instruction(&Instruction::Else);
                compile_async_flow(
                    function,
                    success_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function.instruction(&Instruction::End);
            }
            wasm_ir::Statement::While {
                condition,
                body,
                result,
            } => {
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_expr(function, *condition, context);
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_async_flow(
                    function,
                    body,
                    loop_depth + 2,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: *result,
                    }),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function
                    .instruction(&Instruction::Br(0))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::For {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step,
                body,
            } => {
                compile_for_init(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterable,
                    context,
                );
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_for_has_next(
                    function,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    *iterator_step,
                    context,
                );
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_for_bind_and_advance(
                    function,
                    *binding,
                    *iterable_value,
                    *index_value,
                    *version_value,
                    context,
                );
                compile_async_flow(
                    function,
                    body,
                    loop_depth + 2,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
                        break_destination: None,
                    }),
                    result_global,
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                );
                function
                    .instruction(&Instruction::Br(0))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
            wasm_ir::Statement::ForInit {
                iterable_value,
                index_value,
                version_value,
                iterable,
                ..
            } => compile_for_init(
                function,
                *iterable_value,
                *index_value,
                *version_value,
                *iterable,
                context,
            ),
            wasm_ir::Statement::Evaluate {
                expression,
                discard_result,
            } => {
                compile_expr(function, *expression, context);
                if *discard_result && context.expression_type(*expression) != Type::Never {
                    function.instruction(&Instruction::Drop);
                }
            }
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Fallthrough => {}
        wasm_ir::Terminator::Break(value) => {
            let control = loop_control.expect("checked break expressions belong to loops");
            if let Some(value) = value {
                if let Some(destination) = control.break_destination() {
                    compile_temporary_set(function, destination, *value, context);
                } else {
                    compile_expr(function, *value, &context.erasing_none());
                    if context.expression_type(*value).has_runtime_value() {
                        function.instruction(&Instruction::Drop);
                    }
                }
            }
            control.emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue expressions belong to loops")
                .emit_continue(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::AsyncWhile { header_state, .. } => {
            set_async_state(function, *header_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::AsyncWhileCondition {
            condition,
            body,
            exit_state,
            ..
        } => {
            compile_expr(function, *condition, context);
            function.instruction(&Instruction::If(BlockType::Empty));
            compile_async_flow(
                function,
                body,
                loop_depth + 1,
                loop_control.map(|control| control.nested(1)),
                result_global,
                cancellation_region,
                runtime,
                layout,
                context,
            );
            function.instruction(&Instruction::Else);
            set_async_state(function, *exit_state, context.locals.frame());
            function
                .instruction(&Instruction::Br(loop_depth + 1))
                .instruction(&Instruction::End);
        }
        wasm_ir::Terminator::AsyncFor { header_state, .. } => {
            set_async_state(function, *header_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Return(value) => {
            match context.bare_return {
                BareReturn::AsyncFuture { frame, completion } => {
                    if let Some(value) = value {
                        if let Some((field, _)) = completion {
                            frame.emit(function);
                            compile_expr(function, *value, context);
                            function.instruction(&Instruction::StructSet {
                                struct_type_index: frame.struct_type,
                                field_index: field,
                            });
                        } else {
                            compile_expr(function, *value, &context.erasing_none());
                        }
                    }
                }
                BareReturn::AsyncAttach { .. } => {
                    if let Some(global) = result_global {
                        compile_expr(
                            function,
                            value.expect("layout selection returns a typed layout"),
                            context,
                        );
                        function.instruction(&Instruction::GlobalSet(global));
                    } else {
                        debug_assert!(value.is_none());
                    }
                }
                BareReturn::None | BareReturn::Action(_) => {
                    unreachable!("direct bodies do not use the async state emitter")
                }
            }
            mark_future_complete(function, context.bare_return);
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::Return);
        }
        wasm_ir::Terminator::Suspend {
            mode,
            destination,
            value,
            poll_state,
            resume_state,
            cancellation,
            source,
            ..
        } => {
            if call_target(context.wasm_ir, *value)
                .and_then(resolved_intrinsic)
                .is_some()
            {
                assert_eq!(
                    *cancellation,
                    Some(cancellation_region),
                    "awaited standard-library operation must participate in its body's cancellation region"
                );
            }
            if let Some(debug) = context.debug {
                debug.mark_suspend(function, *source);
            }
            set_async_state(function, *poll_state, context.locals.frame());
            if *mode == SuspensionMode::Await
                && call_target(context.wasm_ir, *value).and_then(resolved_intrinsic)
                    == Some(IntrinsicId::NextTick)
            {
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            } else {
                compile_suspension_poll(
                    function,
                    *mode,
                    *destination,
                    *value,
                    runtime.abi,
                    runtime.strings,
                    layout,
                    context,
                );
            }
            set_async_state(function, *resume_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Retry {
            attempt,
            poll_state,
            cancellation,
            source,
            ..
        } => {
            assert_eq!(
                *cancellation,
                Some(cancellation_region),
                "retry must participate in its body's cancellation region"
            );
            if let Some(debug) = context.debug {
                debug.mark_suspend(function, *source);
            }
            set_async_state(function, *poll_state, context.locals.frame());
            compile_async_flow(
                function,
                attempt,
                loop_depth,
                loop_control,
                result_global,
                cancellation_region,
                runtime,
                layout,
                context,
            );
        }
        wasm_ir::Terminator::RetryComplete {
            value,
            destination,
            resume_state,
        } => {
            compile_retry_poll(function, *destination, *value, layout, context);
            set_async_state(function, *resume_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Throw { error, target } => match target {
            crate::hir::FailureTarget::Return(_) => {
                unreachable!("throw is rejected in async bodies without a retry boundary")
            }
            crate::hir::FailureTarget::Retry { .. } => {
                compile_expr(function, *error, context);
                function
                    .instruction(&Instruction::Drop)
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            }
        },
    }
}
