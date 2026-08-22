/// Emits one fallible state-field polling body. Pointer-backed fields
/// perform host reads and expression-backed fields execute their Wasm-IR plan.
pub(super) fn compile_read(
    field: &StateField,
    function_index: u32,
    abi: &Abi,
    strings: &StringPool,
    lowering: &EmissionContext<'_>,
) -> Function {
    let StateSource::Pointer(path) = &field.source else {
        let StateSource::Expression(_) = &field.source else {
            unreachable!();
        };
        let mut matches = MatchLayout::default();
        let mut local_types = Vec::new();
        let planned = lowering
            .wasm_ir
            .state_expression(field.id)
            .expect("expression-backed state fields have Wasm IR plans");
        let mut planned_locals = HashMap::new();
        plan_wasm_locals(
            &planned.locals,
            &mut planned_locals,
            &mut matches,
            &mut local_types,
            LocalPlanOptions {
                parameter_count: 1,
                semantics: lowering.semantics,
                instance: None,
                include_values: true,
            },
        );
        let mut function = Function::new(
            local_types
                .into_iter()
                .map(|ty| (1, lowering.gc.val_type(ty))),
        );
        let locals = planned_locals;
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
            debug: lowering.debug_emission(function_index),
            function_instance: None,
            loop_control: None,
            bare_return: BareReturn::None,
            materialize_none: true,
        };
        compile_block(&mut function, &planned.entry, &context, None);
        function.instruction(&Instruction::End);
        return function;
    };
    let field_type_id = lowering
        .semantics
        .value_type(field.id)
        .expect("checked state fields have semantic types");
    let field_type = value_type(field.id, lowering.semantics);
    let (memory_type_id, optional) = match lowering.semantics.types().kind(field_type_id) {
        crate::types::TypeKind::Option { layout, value } => (*value, Some(*layout)),
        _ => (field_type_id, None),
    };
    let poll_result = semantic_type(
        lowering
            .semantics
            .state_poll_result(field.id)
            .expect("checked state fields have poll-result types"),
        lowering.semantics,
    );
    let Type::Result(result_type) = poll_result else {
        unreachable!("state poll-result types are Result layouts")
    };
    if let Some(provider) = lowering.semantics.state_provider() {
        let provider = lowering.standard_library.state_provider(provider);
        let Implementation::Intrinsic(direct_read) = lowering
            .standard_library
            .item(provider.direct_read)
            .implementation
        else {
            unreachable!("validated state-provider reads are intrinsic")
        };
        if let Some(contract) = crate::intrinsic_registry::provider_read_contract(direct_read) {
            let field_size = lowering
                .memory
                .layout(memory_type_id, lowering.semantics)
                .expect("provider pointer fields are MemoryReadable")
                .size();
            return compile_provider_direct_read(
                match path.base {
                    crate::ast::PointerPathBase::Absolute(address) => address as u32,
                    crate::ast::PointerPathBase::Module { .. } => {
                        unreachable!("provider direct reads reject module-relative roots")
                    }
                },
                memory_type_id,
                field_type,
                optional,
                field_size,
                result_type,
                contract,
                &path.offsets,
                lowering,
            );
        }
        debug_assert_eq!(direct_read, IntrinsicId::ProcessRead);
    }
    let decoded_string = path.decoder.is_some();
    let mut locals = vec![(1, ValType::I64)];
    if decoded_string {
        locals.push((
            1,
            lowering.gc.val_type(Type::Standard(StdlibTypeId::String)),
        ));
    }
    let mut function = Function::new(locals);
    let address_local = 1;
    let offsets = &path.offsets;
    if let crate::ast::PointerPathBase::Module { name, offset } = &path.base {
        let (ptr, len) = strings.get(name);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::I32Const(ptr as i32))
            .instruction(&Instruction::I32Const(len as i32))
            .instruction(&Instruction::Call(
                abi.function(AbiImportId::ProcessGetModuleAddress),
            ))
            .instruction(&Instruction::LocalTee(address_local))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            "process module was not found",
            lowering.gc,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(address_local))
            .instruction(&Instruction::I64Const(*offset))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    } else if let crate::ast::PointerPathBase::Absolute(address) = path.base {
        function
            .instruction(&Instruction::I64Const(address as i64))
            .instruction(&Instruction::LocalSet(address_local));
    }

    // Resolve the complete pointer chain before either decoding a string or
    // reading an ordinary MemoryReadable value. A decoder changes how the
    // final address is consumed, not how that address is discovered.
    let process_read = ProcessReadEmission {
        abi,
        address_local,
        fallback_ty: field_type,
        optional,
        result_type,
        gc: lowering.gc,
        abi_read: lowering.abi_read,
        read_failure: "process read failed",
    };
    for offset in offsets {
        emit_process_read(&mut function, &process_read, 8);
        function
            .instruction(&Instruction::I32Const(lowering.abi_read.start()))
            .instruction(&Instruction::I64Load(memarg()))
            .instruction(&Instruction::I64Const(*offset))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    }

    if let Some(decoder) = path.decoder {
        let (maximum, helper, failure) = match decoder {
            crate::ast::StateMemoryDecoder::Utf8 { max_bytes, .. } => (
                max_bytes,
                RuntimeHelperId::ReadUtf8String,
                "UTF-8 string could not be read",
            ),
            crate::ast::StateMemoryDecoder::Utf16Le { max_units, .. } => (
                max_units,
                RuntimeHelperId::ReadUtf16LeString,
                "UTF-16LE string could not be read",
            ),
        };
        let string_local = 2;
        function
            .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
            .instruction(&Instruction::LocalGet(address_local))
            .instruction(&Instruction::I32Const(maximum as i32))
            .instruction(&Instruction::Call(
                lowering.runtime_helpers.function(helper),
            ))
            .instruction(&Instruction::LocalTee(string_local))
            .instruction(&Instruction::RefIsNull)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_pointer_read_failure(
            &mut function,
            result_type,
            field_type,
            optional,
            failure,
            lowering.gc,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(string_local));
        emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
        function.instruction(&Instruction::End);
        return function;
    }

    let field_size = lowering
        .memory
        .layout(memory_type_id, lowering.semantics)
        .expect("checked undecoded pointer fields are MemoryReadable")
        .size();
    emit_process_read(&mut function, &process_read, field_size);
    emit_memory_value(
        &mut function,
        memory_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
    );
    emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

/// Emits a fallible per-field candidate transform. Its parameter is the newly
/// read value and its result determines whether the field accepts that value.
pub(super) fn compile_state_transform(
    field: &StateField,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let transform = field
        .transform
        .as_ref()
        .expect("only filtered fields have transform bodies");
    let planned = lowering
        .wasm_ir
        .state_transform(field.id)
        .expect("checked state transforms have Wasm IR plans");
    let field_type = value_type(field.id, lowering.semantics);
    let mut matches = MatchLayout::default();
    let mut local_types = Vec::new();
    let mut planned_locals = HashMap::new();
    plan_wasm_locals(
        &planned.locals,
        &mut planned_locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: 1,
            semantics: lowering.semantics,
            instance: None,
            include_values: true,
        },
    );
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, lowering.gc.val_type(ty))),
    );
    let mut locals = planned_locals;
    locals.insert(transform.value, (0, field_type));
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
        debug: lowering.debug_emission(function_index),
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    compile_block(&mut function, &planned.entry, &context, None);
    function.instruction(&Instruction::End);
    function
}

fn compile_provider_direct_read(
    address: u32,
    memory_type_id: crate::types::TypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    field_size: u32,
    result_type: ResultTypeId,
    contract: crate::intrinsic_registry::ProviderReadContract,
    offsets: &[i64],
    lowering: &EmissionContext<'_>,
) -> Function {
    let mut function = Function::new([(1, ValType::I64), (1, ValType::I32)]);
    let address_local = 1;
    let guest_address_local = 2;
    function
        .instruction(&Instruction::I32Const(address as i32))
        .instruction(&Instruction::LocalSet(guest_address_local));
    for offset in offsets {
        emit_provider_translation(
            &mut function,
            guest_address_local,
            address_local,
            4,
            field_type,
            optional,
            result_type,
            contract,
            lowering,
        );
        emit_process_read(
            &mut function,
            &ProcessReadEmission {
                abi: lowering.abi,
                address_local,
                fallback_ty: field_type,
                optional,
                result_type,
                gc: lowering.gc,
                abi_read: lowering.abi_read,
                read_failure: contract.read_failure,
            },
            4,
        );
        function
            .instruction(&Instruction::I32Const(lowering.abi_read.start()))
            .instruction(&Instruction::I32Load(memarg()))
            .instruction(&Instruction::I32Const(*offset as i32))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(guest_address_local));
    }
    function
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
        .instruction(&Instruction::GlobalGet(
            lowering
                .runtime_globals
                .provider_value
                .expect("provider direct reads require provider storage"),
        ))
        .instruction(&Instruction::LocalGet(guest_address_local))
        .instruction(&Instruction::I32Const(field_size as i32))
        .instruction(&Instruction::Call(
            lowering.runtime_helpers.function(contract.translator),
        ))
        .instruction(&Instruction::LocalTee(address_local))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        &mut function,
        result_type,
        field_type,
        optional,
        contract.invalid_address,
        lowering.gc,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    emit_process_read(
        &mut function,
        &ProcessReadEmission {
            abi: lowering.abi,
            address_local,
            fallback_ty: field_type,
            optional,
            result_type,
            gc: lowering.gc,
            abi_read: lowering.abi_read,
            read_failure: contract.read_failure,
        },
        field_size,
    );
    emit_memory_value(
        &mut function,
        memory_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
    );
    emit_pointer_read_success(&mut function, result_type, optional, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

fn emit_provider_translation(
    function: &mut Function,
    guest_address_local: u32,
    address_local: u32,
    size: u32,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    result_type: ResultTypeId,
    contract: crate::intrinsic_registry::ProviderReadContract,
    lowering: &EmissionContext<'_>,
) {
    function
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
        .instruction(&Instruction::GlobalGet(
            lowering
                .runtime_globals
                .provider_value
                .expect("provider direct reads require provider storage"),
        ))
        .instruction(&Instruction::LocalGet(guest_address_local))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(
            lowering.runtime_helpers.function(contract.translator),
        ))
        .instruction(&Instruction::LocalTee(address_local))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        function,
        result_type,
        field_type,
        optional,
        contract.invalid_address,
        lowering.gc,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

struct ProcessReadEmission<'a> {
    abi: &'a Abi,
    address_local: u32,
    fallback_ty: Type,
    optional: Option<crate::ast::OptionTypeId>,
    result_type: ResultTypeId,
    gc: &'a GcLayout,
    abi_read: memory_plan::AbiReadScratch,
    read_failure: &'a str,
}

fn emit_process_read(function: &mut Function, emission: &ProcessReadEmission<'_>, size: u32) {
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(emission.address_local))
        .instruction(&Instruction::I32Const(emission.abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(
            emission.abi.function(AbiImportId::ProcessRead),
        ))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_pointer_read_failure(
        function,
        emission.result_type,
        emission.fallback_ty,
        emission.optional,
        emission.read_failure,
        emission.gc,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

fn emit_pointer_read_failure(
    function: &mut Function,
    result_type: ResultTypeId,
    field_type: Type,
    optional: Option<crate::ast::OptionTypeId>,
    message: &str,
    gc: &GcLayout,
) {
    if let Some(option) = optional {
        function.instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.index(Type::Option(option)),
        )));
        emit_result_success(function, result_type, gc);
    } else {
        emit_result_error(function, result_type, field_type, message, gc);
    }
}

fn emit_pointer_read_success(
    function: &mut Function,
    result_type: ResultTypeId,
    optional: Option<crate::ast::OptionTypeId>,
    gc: &GcLayout,
) {
    if let Some(option) = optional {
        function.instruction(&Instruction::StructNew(gc.index(Type::Option(option))));
    }
    emit_result_success(function, result_type, gc);
}

pub(super) fn compile_user_function(
    declaration: &FunctionDecl,
    instance: &crate::semantic::FunctionInstance,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let wasm_body = lowering
        .wasm_ir
        .body(BodyOwner::Function(instance.clone()))
        .expect("checked functions have Wasm IR bodies");
    let mut locals = HashMap::new();
    let mut physical_parameter_count = 0;
    for parameter in &declaration.params {
        let ty = semantic_type(
            lowering.semantics.specialize_type(
                instance,
                lowering
                    .semantics
                    .value_type(parameter.id)
                    .expect("checked parameters have types"),
            ),
            lowering.semantics,
        );
        let index = if !ty.has_runtime_value() {
            u32::MAX
        } else {
            let index = physical_parameter_count;
            physical_parameter_count += 1;
            index
        };
        locals.insert(parameter.id, (index, ty));
    }
    let mut local_types = Vec::new();
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: physical_parameter_count,
            semantics: lowering.semantics,
            instance: Some(instance),
            include_values: true,
        },
    );
    if let Some(debug) = lowering.debug {
        for parameter in &declaration.params {
            let (local, ty) = locals[&parameter.id];
            debug.register_variable(function_index, parameter.id, local, ty, true);
        }
        for (&value, &(local, ty)) in &locals {
            if !declaration
                .params
                .iter()
                .any(|parameter| parameter.id == value)
            {
                debug.register_variable(function_index, value, local, ty, false);
            }
        }
    }
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, lowering.gc.val_type(ty))),
    );
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
        debug: lowering.debug_emission(function_index),
        function_instance: Some(instance),
        loop_control: None,
        bare_return: BareReturn::None,
        materialize_none: true,
    };
    compile_block(&mut function, &wasm_body.entry, &context, None);
    let result = semantic_type(
        lowering.semantics.specialize_type(
            instance,
            lowering
                .semantics
                .function_completion(declaration.id)
                .expect("checked functions have result types"),
        ),
        lowering.semantics,
    );
    if result.has_runtime_value() {
        function.instruction(&Instruction::Unreachable);
    }
    function.instruction(&Instruction::End);
    function
}

pub(super) fn compile_action(
    action: &Action,
    function_index: u32,
    lowering: &EmissionContext<'_>,
) -> Function {
    let wasm_body = lowering
        .wasm_ir
        .body(BodyOwner::Action(action.kind))
        .expect("checked actions have Wasm IR bodies");
    let mut locals = HashMap::new();
    let mut local_types = Vec::new();
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        LocalPlanOptions {
            parameter_count: if matches!(action.kind, ActionKind::Setup | ActionKind::OnDetach) {
                0
            } else {
                2
            },
            semantics: lowering.semantics,
            instance: None,
            include_values: true,
        },
    );
    if let Some(debug) = lowering.debug {
        for (&value, &(local, ty)) in &locals {
            debug.register_variable(function_index, value, local, ty, false);
        }
    }
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, lowering.gc.val_type(ty))),
    );
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
        debug: lowering.debug_emission(function_index),
        function_instance: None,
        loop_control: None,
        bare_return: BareReturn::Action(action.kind),
        materialize_none: true,
    };
    compile_block(&mut function, &wasm_body.entry, &context, Some(action.kind));
    emit_action_default(&mut function, action.kind, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

pub(super) fn emit_action_default(function: &mut Function, action: ActionKind, gc: &GcLayout) {
    match action {
        ActionKind::Start | ActionKind::Split | ActionKind::Reset => {
            function.instruction(&Instruction::I32Const(0));
        }
        ActionKind::WhileAttached => {
            function.instruction(&Instruction::I32Const(1));
        }
        ActionKind::IsLoading => {
            function.instruction(&Instruction::I32Const(-1));
        }
        ActionKind::GameTime => {
            function.instruction(&Instruction::RefNull(HeapType::Concrete(
                gc.standard_index(StdlibTypeId::Duration),
            )));
        }
        ActionKind::Setup
        | ActionKind::OnDetach
        | ActionKind::OnAttach
        | ActionKind::OnStateReady => {}
    }
}

pub(super) struct LocalPlanOptions<'a> {
    pub(super) parameter_count: u32,
    pub(super) semantics: &'a SemanticModel,
    pub(super) instance: Option<&'a crate::semantic::FunctionInstance>,
    pub(super) include_values: bool,
}

pub(super) fn plan_wasm_locals(
    planned: &[wasm_ir::Local],
    locals: &mut HashMap<ValueId, (u32, Type)>,
    matches: &mut MatchLayout,
    types: &mut Vec<Type>,
    options: LocalPlanOptions<'_>,
) {
    for local in planned {
        if matches!(local.purpose, LocalPurpose::Value(_)) && !options.include_values {
            continue;
        }
        let index = options.parameter_count + types.len() as u32;
        let ty = semantic_type(
            options.instance.map_or(local.ty, |instance| {
                options.semantics.specialize_type(instance, local.ty)
            }),
            options.semantics,
        );
        if ty == Type::Never {
            match local.purpose {
                LocalPurpose::Value(value) => {
                    locals.insert(value, (u32::MAX, ty));
                }
                LocalPurpose::Temporary(temporary) => {
                    matches.temporaries.insert(temporary, (u32::MAX, ty));
                }
                LocalPurpose::MatchValue(expression) => {
                    matches.values.insert(expression, u32::MAX);
                }
                LocalPurpose::FallbackValue(expression) => {
                    matches.fallback_values.insert(expression, u32::MAX);
                }
                LocalPurpose::IntrinsicScratch { expression, .. } => {
                    matches
                        .intrinsic_temps
                        .entry(expression)
                        .or_default()
                        .push(u32::MAX);
                }
                LocalPurpose::SuspensionScratch(expression) => {
                    matches.suspension_temps.insert(expression, u32::MAX);
                }
            }
            continue;
        }
        if ty == Type::None
            && let LocalPurpose::Value(value) = local.purpose
        {
            locals.insert(value, (u32::MAX, ty));
            continue;
        }
        types.push(ty);
        match local.purpose {
            LocalPurpose::Value(value) => {
                locals.insert(value, (index, ty));
            }
            LocalPurpose::Temporary(temporary) => {
                matches.temporaries.insert(temporary, (index, ty));
            }
            LocalPurpose::MatchValue(expression) => {
                matches.values.insert(expression, index);
            }
            LocalPurpose::FallbackValue(expression) => {
                matches.fallback_values.insert(expression, index);
            }
            LocalPurpose::IntrinsicScratch { expression, .. } => {
                matches
                    .intrinsic_temps
                    .entry(expression)
                    .or_default()
                    .push(index);
            }
            LocalPurpose::SuspensionScratch(expression) => {
                matches.suspension_temps.insert(expression, index);
            }
        }
    }
}
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    abi::AbiImportId,
    ast::{Action, ActionKind, FunctionDecl, ResultTypeId, StateField, StateSource, ValueId},
    intrinsic_registry::RuntimeHelperId,
    semantic::SemanticModel,
    stdlib::{Implementation, IntrinsicId, StdlibTypeId},
    wasm_ir::{self, BodyOwner, LocalPurpose},
};

use super::{
    GcLayout, Type,
    async_frame::AsyncFrameLayout,
    context::EmissionContext,
    data_plan::StringPool,
    emit_default, emit_memory_value, emit_result_error, emit_result_success,
    expression::{BareReturn, ExprContext, LocalStorage, MatchLayout, compile_block},
    imports::Abi,
    memarg, memory_plan, semantic_type, value_type,
};

pub(super) fn compile_async_function_init(
    declaration: &FunctionDecl,
    instance: &crate::semantic::FunctionInstance,
    layout: &AsyncFrameLayout,
    lowering: &EmissionContext<'_>,
) -> Function {
    let mut next_parameter = 0;
    let parameter_locals = declaration
        .params
        .iter()
        .map(|parameter| {
            let ty = semantic_type(
                lowering.semantics.specialize_type(
                    instance,
                    lowering
                        .semantics
                        .value_type(parameter.id)
                        .expect("checked parameters have types"),
                ),
                lowering.semantics,
            );
            let local = ty.has_runtime_value().then(|| {
                let local = next_parameter;
                next_parameter += 1;
                local
            });
            (parameter.id, local)
        })
        .collect::<HashMap<_, _>>();
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(
            lowering.gc.function_frame_tag(instance) as i32,
        ));
    for (position, ty) in layout.types.iter().copied().enumerate() {
        let field = position as u32 + layout.base_fields;
        if let Some(parameter) = declaration.params.iter().find_map(|parameter| {
            layout
                .fields
                .get(&parameter.id)
                .is_some_and(|(candidate, _)| *candidate == field)
                .then_some(parameter.id)
        }) {
            if let Some(local) = parameter_locals[&parameter] {
                function.instruction(&Instruction::LocalGet(local));
            } else {
                emit_default(&mut function, ty, lowering.gc);
            }
        } else {
            emit_default(&mut function, ty, lowering.gc);
        }
    }
    function
        .instruction(&Instruction::StructNew(
            lowering.gc.function_frame_index(instance),
        ))
        .instruction(&Instruction::End);
    function
}
