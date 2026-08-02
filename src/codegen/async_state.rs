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
    LocalPlanOptions, Type,
    async_frame::{
        AsyncFrameLayout, AsyncFrameRef, AsyncFrameSource, IntrinsicFutureInstance,
        IntrinsicFutureLayout,
    },
    call_target,
    context::AttachContext,
    data_plan::{SignaturePool, StringPool},
    emit_memory_value, emit_string_literal, emit_typed_struct_get,
    expression::{
        BareReturn, ExprContext, IntrinsicCapture, LocalStorage, LoopControl, MatchLayout,
        compile_assignment, compile_expr, compile_fallback_condition, compile_for_bind_and_advance,
        compile_for_has_next, compile_for_init, compile_receiver, compile_statement_pattern,
        compile_temporary_set, store_match_binding,
    },
    imports::Abi,
    memarg, plan_wasm_locals, resolved_intrinsic, semantic_type, unity_layout,
};

pub(super) fn compile_async_attach(
    action: &Action,
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
        wasm_body,
        layout,
        runtime,
        frame,
        None,
        BareReturn::AsyncAttach,
        result_global,
    )
}

pub(super) fn compile_async_function_poll(
    instance: &FunctionInstance,
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
        wasm_body,
        layout,
        runtime,
        frame,
        Some(instance),
        BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
        None,
    )
}

pub(super) fn compile_intrinsic_future_poll(
    instance: &IntrinsicFutureInstance,
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
            instance: instance.owner.as_ref(),
            include_values: false,
        },
    );
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, runtime.lowering.gc.val_type(ty))),
    );
    let empty_values = HashMap::new();
    let empty_temporaries = HashMap::new();
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
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
        intrinsic_futures: runtime.lowering.intrinsic_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        records: runtime.lowering.records,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        intrinsic_capture: Some(IntrinsicCapture { frame, layout }),
        function_instance: instance.owner.as_ref(),
        loop_control: None,
        bare_return: BareReturn::AsyncFuture {
            frame,
            completion: layout.completion,
        },
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
        runtime.signatures,
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
    wasm_body: &wasm_ir::Body,
    layout: &AsyncFrameLayout,
    runtime: &AttachContext<'_>,
    frame: AsyncFrameRef,
    function_instance: Option<&FunctionInstance>,
    bare_return: BareReturn,
    result_global: Option<u32>,
) -> Function {
    let cancellation_region = wasm_body
        .cancellation_region
        .expect("onAttach is owned by the process-lifetime cancellation region");
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut planned_locals = HashMap::new();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut planned_locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1,
            semantics: runtime.lowering.semantics,
            instance: function_instance,
            include_values: true,
        },
    );
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, runtime.lowering.gc.val_type(ty))),
    );
    let context = ExprContext {
        standard_library: runtime.lowering.standard_library,
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
        intrinsic_futures: runtime.lowering.intrinsic_futures,
        display_functions: runtime.lowering.display_functions,
        equality_functions: runtime.lowering.equality_functions,
        records: runtime.lowering.records,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        abi_read: runtime.lowering.abi_read,
        matches: &matches,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        async_frames: runtime.lowering.async_frames,
        intrinsic_capture: None,
        function_instance,
        loop_control: None,
        bare_return,
    };

    let mut states = (0..wasm_body.async_state_count)
        .map(|_| None)
        .collect::<Vec<_>>();
    states[wasm_ir::AsyncStateId::ENTRY.index() as usize] = Some(AsyncState::Block {
        block: &wasm_body.entry,
        loop_targets: None,
    });
    collect_async_states(&wasm_body.entry, &mut states, None);
    debug_assert!(states.iter().all(Option::is_some));

    function.instruction(&Instruction::Loop(BlockType::Empty));
    for (pc, state) in states.into_iter().enumerate() {
        frame.emit(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: frame.struct_type,
                field_index: 0,
            })
            .instruction(&Instruction::I32Const(pc as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));

        match state.expect("every async state is assigned during lowering") {
            AsyncState::Block {
                block,
                loop_targets,
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
                body,
                header_state,
                exit_state,
            } => {
                compile_for_has_next(&mut function, iterable_value, index_value, &context);
                function.instruction(&Instruction::If(BlockType::Empty));
                compile_for_bind_and_advance(
                    &mut function,
                    binding,
                    iterable_value,
                    index_value,
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
                    runtime.signatures,
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

#[allow(clippy::too_many_arguments)]
fn compile_suspension_poll(
    function: &mut Function,
    mode: SuspensionMode,
    destination: wasm_ir::SuspensionDestination,
    value: ExprId,
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
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
    if !matches!(
        &value_expression.kind,
        wasm_ir::ExpressionKind::Call {
            target: wasm_ir::CallTarget::Intrinsic { .. },
            ..
        }
    ) {
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
    let unity_module_local = primary_scratch;
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
            let wasm_ir::ExpressionKind::Signature(signature) = &context
                .wasm_ir
                .expression(args[2])
                .expect("scan signature belongs to Wasm IR")
                .kind
            else {
                unreachable!();
            };
            let entry = signatures.get(signature);
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            compile_expr(function, args[1], context);
            function
                .instruction(&Instruction::I32Const(entry.needle as i32))
                .instruction(&Instruction::I32Const(entry.mask as i32))
                .instruction(&Instruction::I32Const(entry.len as i32))
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ScanProcessRange),
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
        Some(IntrinsicId::UnityIl2Cpp) => {
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::UnityAttach),
                ))
                .instruction(&Instruction::LocalTee(unity_module_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(destination) {
                context.locals.frame().emit(function);
                function
                    .instruction(&Instruction::LocalGet(unity_module_local))
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
            let wasm_ir::ExpressionKind::Signature(signature) = &context
                .wasm_ir
                .expression(args[0])
                .expect("scan signature belongs to Wasm IR")
                .kind
            else {
                unreachable!();
            };
            let entry = signatures.get(signature);
            function.instruction(&Instruction::GlobalGet(context.runtime_globals.process));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context
                        .gc
                        .standard_field_index(StdlibFieldId::ModuleAddress),
                });
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: context.gc.standard_index(StdlibTypeId::Module),
                    field_index: context.gc.standard_field_index(StdlibFieldId::ModuleSize),
                })
                .instruction(&Instruction::I32Const(entry.needle as i32))
                .instruction(&Instruction::I32Const(entry.mask as i32))
                .instruction(&Instruction::I32Const(entry.len as i32))
                .instruction(&Instruction::Call(
                    context
                        .runtime_helpers
                        .function(RuntimeHelperId::ScanProcessRange),
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
    let intrinsic_candidates = context
        .async_frames
        .intrinsics()
        .filter(|(_, layout)| layout.future == child_type)
        .collect::<Vec<_>>();
    assert!(
        !source_candidates.is_empty() || !intrinsic_candidates.is_empty(),
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
            let (completion_field, completion_type) = child_layout
                .completion
                .expect("value-producing async callees have completion slots");
            debug_assert_eq!(destination_type, completion_type);
            parent.emit(function);
            emit_child_frame(function, parent, child_field, child_frame);
            emit_typed_struct_get(function, child_frame, completion_field, completion_type);
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
            let (completion_field, completion_type) = child_layout
                .completion
                .expect("value-producing intrinsic futures have completion slots");
            debug_assert_eq!(destination_type, completion_type);
            parent.emit(function);
            emit_child_frame(function, parent, child_field, child_frame);
            emit_typed_struct_get(function, child_frame, completion_field, completion_type);
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

    let names = &context.state.processes;
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

    let names = &context.state.processes;
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
    },
    ForHeader {
        binding: ValueId,
        iterable_value: ValueId,
        index_value: ValueId,
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
    },
}

#[derive(Clone, Copy)]
struct AsyncLoopTargets {
    break_state: wasm_ir::AsyncStateId,
    continue_state: wasm_ir::AsyncStateId,
}

impl AsyncLoopTargets {
    fn control(self, dispatcher_depth: u32) -> LoopControl {
        LoopControl::Async {
            break_state: self.break_state,
            continue_state: self.continue_state,
            dispatcher_depth,
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
            | wasm_ir::Statement::StoreTemporary { .. }
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
            continuation,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Poll {
                mode: *mode,
                destination: *destination,
                value: *value,
                resume_state: *resume_state,
                cancellation: *cancellation,
            });
            states[resume_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
            });
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::AsyncWhile {
            header,
            continuation,
            header_state,
            exit_state,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
            };
            states[header_state.index() as usize] = Some(AsyncState::Block {
                block: header,
                loop_targets: Some(inner_targets),
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
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
            body,
            continuation,
            header_state,
            exit_state,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
            };
            states[header_state.index() as usize] = Some(AsyncState::ForHeader {
                binding: *binding,
                iterable_value: *iterable_value,
                index_value: *index_value,
                body,
                header_state: *header_state,
                exit_state: *exit_state,
            });
            states[exit_state.index() as usize] = Some(AsyncState::Block {
                block: continuation,
                loop_targets,
            });
            collect_async_states(body, states, Some(inner_targets));
            collect_async_states(continuation, states, loop_targets);
        }
        wasm_ir::Terminator::Fallthrough
        | wasm_ir::Terminator::Break
        | wasm_ir::Terminator::Continue
        | wasm_ir::Terminator::Return(_)
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
            wasm_ir::Statement::Store {
                target,
                operation,
                value,
                ..
            } => {
                compile_assignment(function, *target, operation.as_ref(), *value, context);
            }
            wasm_ir::Statement::StoreTemporary { target, value } => {
                compile_temporary_set(function, *target, *value, context);
            }
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
            wasm_ir::Statement::While { condition, body } => {
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
                iterable,
                body,
            } => {
                compile_for_init(function, *iterable_value, *index_value, *iterable, context);
                function
                    .instruction(&Instruction::Block(BlockType::Empty))
                    .instruction(&Instruction::Loop(BlockType::Empty));
                compile_for_has_next(function, *iterable_value, *index_value, context);
                function
                    .instruction(&Instruction::I32Eqz)
                    .instruction(&Instruction::BrIf(1));
                compile_for_bind_and_advance(
                    function,
                    *binding,
                    *iterable_value,
                    *index_value,
                    context,
                );
                compile_async_flow(
                    function,
                    body,
                    loop_depth + 2,
                    Some(LoopControl::Branch {
                        break_depth: 1,
                        continue_depth: 0,
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
                iterable,
                ..
            } => compile_for_init(function, *iterable_value, *index_value, *iterable, context),
            wasm_ir::Statement::Evaluate {
                expression,
                discard_result,
            } => {
                compile_expr(function, *expression, context);
                if *discard_result {
                    function.instruction(&Instruction::Drop);
                }
            }
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Fallthrough => {}
        wasm_ir::Terminator::Break => {
            loop_control
                .expect("checked break statements belong to loops")
                .emit_break(function, context.locals.continuation_frame());
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue statements belong to loops")
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
                        let (field, _) =
                            completion.expect("value-returning futures have completion slots");
                        frame.emit(function);
                        compile_expr(function, *value, context);
                        function.instruction(&Instruction::StructSet {
                            struct_type_index: frame.struct_type,
                            field_index: field,
                        });
                    }
                }
                BareReturn::AsyncAttach => {
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
                BareReturn::Void | BareReturn::Action(_) => {
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
                    runtime.signatures,
                    layout,
                    context,
                );
            }
            set_async_state(function, *resume_state, context.locals.frame());
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Throw { .. } => {
            unreachable!("throw is rejected in onAttach until it has a result boundary")
        }
    }
}
