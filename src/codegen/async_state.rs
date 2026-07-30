//! Wasm-IR async state-machine, suspension, retry, and cancellation emission.

use super::expression::LoopControl;
use super::*;

pub(super) fn compile_async_attach(
    action: &Action,
    layout: &AsyncFrameLayout,
    runtime: &RuntimeContext<'_>,
) -> Function {
    let wasm_body = runtime
        .lowering
        .wasm_ir
        .body(BodyOwner::Action(action.kind))
        .expect("checked actions have Wasm IR bodies");
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
        1,
        runtime.lowering.semantics,
        true,
    );
    let module_address_local = 1 + local_types.len() as u32;
    local_types.push(Type::U64);
    let module_size_local = 1 + local_types.len() as u32;
    local_types.push(Type::U64);
    let unity_module_local = 1 + local_types.len() as u32;
    local_types.push(Type::UnityModule);
    let unity_image_local = 1 + local_types.len() as u32;
    local_types.push(Type::UnityImage);
    let unity_class_local = 1 + local_types.len() as u32;
    local_types.push(Type::UnityClass);
    let unity_field_local = 1 + local_types.len() as u32;
    local_types.push(Type::UnityField);
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, runtime.lowering.gc.val_type(ty))),
    );
    let pattern_bindings = HashMap::new();
    let context = ExprContext {
        abi: runtime.abi,
        state: runtime.lowering.state,
        locals: LocalStorage::Hybrid {
            wasm: &planned_locals,
            frame: &layout.fields,
        },
        globals: runtime.lowering.globals,
        global_types: runtime.lowering.global_types,
        settings: runtime.lowering.settings,
        stdlib: runtime.lowering.stdlib,
        functions: runtime.lowering.functions,
        equality_functions: runtime.lowering.equality_functions,
        records: runtime.lowering.records,
        enums: runtime.lowering.enums,
        arrays: runtime.lowering.arrays,
        memory: runtime.lowering.memory,
        matches: &matches,
        pattern_bindings: &pattern_bindings,
        semantics: runtime.lowering.semantics,
        wasm_ir: runtime.lowering.wasm_ir,
        gc: runtime.lowering.gc,
        loop_control: None,
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
        emit_async_frame_ref(&mut function);
        function
            .instruction(&Instruction::StructGet {
                struct_type_index: ASYNC_FRAME_TYPE,
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
                cancellation_region,
                runtime,
                layout,
                &context,
                module_address_local,
                module_size_local,
                unity_module_local,
                unity_image_local,
                unity_class_local,
                unity_field_local,
            ),
            AsyncState::LoopHeader {
                condition,
                body,
                header_state,
                exit_state,
            } => {
                compile_expr(&mut function, condition, &context);
                function.instruction(&Instruction::If(BlockType::Empty));
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
                    cancellation_region,
                    runtime,
                    layout,
                    &context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
                );
                function.instruction(&Instruction::Else);
                set_async_state(&mut function, exit_state);
                function
                    .instruction(&Instruction::Br(2))
                    .instruction(&Instruction::End);
            }
            AsyncState::Poll {
                mode,
                binding,
                value,
                resume_state,
                cancellation,
            } => {
                assert_eq!(
                    cancellation,
                    Some(cancellation_region),
                    "awaited standard-library operation must participate in its body's cancellation region"
                );
                compile_suspension_poll(
                    &mut function,
                    mode,
                    binding,
                    value,
                    runtime.abi,
                    runtime.strings,
                    runtime.signatures,
                    layout,
                    &context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
                );
                set_async_state(&mut function, resume_state);
                function.instruction(&Instruction::Br(1));
            }
        }
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

#[allow(clippy::too_many_arguments)]
fn compile_suspension_poll(
    function: &mut Function,
    mode: SuspensionMode,
    binding: Option<ValueId>,
    value: ExprId,
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
    module_address_local: u32,
    module_size_local: u32,
    unity_module_local: u32,
    unity_image_local: u32,
    unity_class_local: u32,
    unity_field_local: u32,
) {
    if mode == SuspensionMode::Retry {
        compile_retry_poll(function, binding, value, layout, context);
        return;
    }
    let value_expression = context
        .wasm_ir
        .expression(value)
        .expect("await value belongs to Wasm IR");
    let wasm_ir::ExpressionKind::Call {
        target,
        arguments: args,
    } = &value_expression.kind
    else {
        unreachable!();
    };
    match resolved_intrinsic(target) {
        Some(IntrinsicId::NextTick) => {}
        Some(IntrinsicId::ProcessModule) => {
            let wasm_ir::ExpressionKind::String(name) = &context
                .wasm_ir
                .expression(args[0])
                .expect("module name belongs to Wasm IR")
                .kind
            else {
                unreachable!();
            };
            let (ptr, len) = strings.get(name);
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::I32Const(ptr as i32))
                .instruction(&Instruction::I32Const(len as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::ProcessGetModuleAddress),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::I32Const(ptr as i32))
                .instruction(&Instruction::I32Const(len as i32))
                .instruction(&Instruction::Call(
                    abi.function(AbiImportId::ProcessGetModuleSize),
                ))
                .instruction(&Instruction::LocalTee(module_size_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::LocalGet(module_size_local))
                    .instruction(&Instruction::StructNew(MODULE_TYPE))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessRead) => {
            let read_type_id = match target {
                ResolvedCall::StandardLibrary { type_arguments, .. } => type_arguments[0],
                _ => unreachable!("process.read must resolve to its standard-library item"),
            };
            let read_type = semantic_type(read_type_id, context.semantics);
            let read_size = context
                .memory
                .layout(read_type_id, context.semantics)
                .expect("checked process reads are MemoryReadable")
                .size();
            if let Some((_, stored_type)) = layout.field(binding) {
                emit_async_frame_ref(function);
                debug_assert_eq!(stored_type, read_type);
            }
            function.instruction(&Instruction::LocalGet(0));
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::I32Const(read_size as i32))
                .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if read_type == Type::Address {
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::I64Load(memarg()))
                    .instruction(&Instruction::I64Eqz)
                    .instruction(&Instruction::If(BlockType::Empty))
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return)
                    .instruction(&Instruction::End);
            }
            if let Some((field, _)) = layout.field(binding) {
                emit_memory_value(
                    function,
                    read_type_id,
                    0,
                    context.memory,
                    context.semantics,
                    context.gc,
                );
                function.instruction(&Instruction::StructSet {
                    struct_type_index: ASYNC_FRAME_TYPE,
                    field_index: field,
                });
            }
        }
        Some(IntrinsicId::ProcessFollow) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_expr(function, args[0], context);
            compile_expr(function, args[1], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::FollowAddress),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
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
            function.instruction(&Instruction::LocalGet(0));
            compile_expr(function, args[0], context);
            compile_expr(function, args[1], context);
            function
                .instruction(&Instruction::I32Const(entry.needle as i32))
                .instruction(&Instruction::I32Const(entry.mask as i32))
                .instruction(&Instruction::I32Const(entry.len as i32))
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::ScanProcessRange),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::ProcessReadRelative32) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::ReadRelative32),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityIl2Cpp) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::UnityAttach),
                ))
                .instruction(&Instruction::LocalTee(unity_module_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(unity_module_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityModuleImage) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::UnityGetImage),
                ))
                .instruction(&Instruction::LocalTee(unity_image_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(unity_image_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityImageClass) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::UnityGetClass),
                ))
                .instruction(&Instruction::LocalTee(unity_class_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(unity_class_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassFieldAny) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::UnityGetFieldAny),
                ))
                .instruction(&Instruction::LocalTee(unity_field_local))
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(unity_field_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassField) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::UnityGetFieldOffset),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::I64Const(1))
                    .instruction(&Instruction::I64Sub)
                    .instruction(&Instruction::I32WrapI64)
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassStaticInstance) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            compile_expr(function, args[0], context);
            function
                .instruction(&Instruction::Call(
                    context
                        .stdlib
                        .helper(GeneratedHelper::UnityGetStaticInstance),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        Some(IntrinsicId::UnityClassStaticTable) => {
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: UNITY_CLASS_TYPE,
                    field_index: 0,
                })
                .instruction(&Instruction::I64Const(0xb8))
                .instruction(&Instruction::I64Add)
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::I32Const(8))
                .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
                .instruction(&Instruction::I32Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End)
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::I64Load(memarg()))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
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
            function.instruction(&Instruction::LocalGet(0));
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: MODULE_TYPE,
                    field_index: 0,
                });
            compile_receiver(function, target, context);
            function
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: MODULE_TYPE,
                    field_index: 1,
                })
                .instruction(&Instruction::I32Const(entry.needle as i32))
                .instruction(&Instruction::I32Const(entry.mask as i32))
                .instruction(&Instruction::I32Const(entry.len as i32))
                .instruction(&Instruction::Call(
                    context.stdlib.helper(GeneratedHelper::ScanProcessRange),
                ))
                .instruction(&Instruction::LocalTee(module_address_local))
                .instruction(&Instruction::I64Eqz)
                .instruction(&Instruction::If(BlockType::Empty))
                .instruction(&Instruction::I32Const(0))
                .instruction(&Instruction::Return)
                .instruction(&Instruction::End);
            if let Some((field, _)) = layout.field(binding) {
                emit_async_frame_ref(function);
                function
                    .instruction(&Instruction::LocalGet(module_address_local))
                    .instruction(&Instruction::StructSet {
                        struct_type_index: ASYNC_FRAME_TYPE,
                        field_index: field,
                    });
            }
        }
        _ => unreachable!("type checking only permits awaitable builtins"),
    }
}

fn compile_retry_poll(
    function: &mut Function,
    binding: Option<ValueId>,
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

    if let Some((field, stored_type)) = frame.field(binding) {
        debug_assert_eq!(stored_type, semantic_type(*result_value, context.semantics));
        emit_async_frame_ref(function);
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
            struct_type_index: ASYNC_FRAME_TYPE,
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
    LoopHeader {
        condition: ExprId,
        body: &'a wasm_ir::Block,
        header_state: wasm_ir::AsyncStateId,
        exit_state: wasm_ir::AsyncStateId,
    },
    Poll {
        mode: SuspensionMode,
        binding: Option<ValueId>,
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
            wasm_ir::Statement::While { body, .. } => {
                collect_async_states(body, states, loop_targets)
            }
            wasm_ir::Statement::Store { .. } | wasm_ir::Statement::Evaluate { .. } => {}
        }
    }
    match &block.terminator {
        wasm_ir::Terminator::Suspend {
            mode,
            binding,
            value,
            poll_state,
            resume_state,
            cancellation,
            continuation,
            ..
        } => {
            states[poll_state.index() as usize] = Some(AsyncState::Poll {
                mode: *mode,
                binding: *binding,
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
            condition,
            body,
            continuation,
            header_state,
            exit_state,
        } => {
            let inner_targets = AsyncLoopTargets {
                break_state: *exit_state,
                continue_state: *header_state,
            };
            states[header_state.index() as usize] = Some(AsyncState::LoopHeader {
                condition: *condition,
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

fn set_async_state(function: &mut Function, state: wasm_ir::AsyncStateId) {
    emit_async_frame_ref(function);
    function
        .instruction(&Instruction::I32Const(state.index() as i32))
        .instruction(&Instruction::StructSet {
            struct_type_index: ASYNC_FRAME_TYPE,
            field_index: 0,
        });
}

#[allow(clippy::too_many_arguments)]
fn compile_async_flow(
    function: &mut Function,
    block: &wasm_ir::Block,
    loop_depth: u32,
    loop_control: Option<LoopControl>,
    cancellation_region: wasm_ir::CancellationRegion,
    runtime: &RuntimeContext<'_>,
    layout: &AsyncFrameLayout,
    context: &ExprContext<'_>,
    module_address_local: u32,
    module_size_local: u32,
    unity_module_local: u32,
    unity_image_local: u32,
    unity_class_local: u32,
    unity_field_local: u32,
) {
    let expression_context = ExprContext {
        loop_control,
        ..*context
    };
    let context = &expression_context;
    for statement in &block.statements {
        match statement {
            wasm_ir::Statement::Store { target, op, value } => {
                compile_assignment(function, *target, *op, *value, context);
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
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
                );
                function.instruction(&Instruction::Else);
                compile_async_flow(
                    function,
                    else_block,
                    loop_depth + 1,
                    loop_control.map(|control| control.nested(1)),
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
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
                    cancellation_region,
                    runtime,
                    layout,
                    context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
                );
                function
                    .instruction(&Instruction::Br(0))
                    .instruction(&Instruction::End)
                    .instruction(&Instruction::End);
            }
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
                .emit_break(function);
        }
        wasm_ir::Terminator::Continue => {
            loop_control
                .expect("checked continue statements belong to loops")
                .emit_continue(function);
        }
        wasm_ir::Terminator::AsyncWhile { header_state, .. } => {
            set_async_state(function, *header_state);
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Return(value) => {
            debug_assert!(value.is_none());
            function
                .instruction(&Instruction::I32Const(1))
                .instruction(&Instruction::Return);
        }
        wasm_ir::Terminator::Suspend {
            mode,
            binding,
            value,
            poll_state,
            resume_state,
            cancellation,
            ..
        } => {
            assert_eq!(
                *cancellation,
                Some(cancellation_region),
                "awaited standard-library operation must participate in its body's cancellation region"
            );
            set_async_state(function, *poll_state);
            if *mode == SuspensionMode::Await
                && resolved_intrinsic(call_target(context.wasm_ir, *value))
                    == Some(IntrinsicId::NextTick)
            {
                function
                    .instruction(&Instruction::I32Const(0))
                    .instruction(&Instruction::Return);
            } else {
                compile_suspension_poll(
                    function,
                    *mode,
                    *binding,
                    *value,
                    runtime.abi,
                    runtime.strings,
                    runtime.signatures,
                    layout,
                    context,
                    module_address_local,
                    module_size_local,
                    unity_module_local,
                    unity_image_local,
                    unity_class_local,
                    unity_field_local,
                );
            }
            set_async_state(function, *resume_state);
            function.instruction(&Instruction::Br(loop_depth));
        }
        wasm_ir::Terminator::Throw { .. } => {
            unreachable!("throw is rejected in onAttach until it has a result boundary")
        }
    }
}
