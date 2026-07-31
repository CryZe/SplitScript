/// Emits one transactional state-field polling body. Pointer-backed fields
/// perform host reads and expression-backed fields execute their Wasm-IR plan.
pub(super) fn compile_read(
    field: &StateField,
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
            1,
            lowering.semantics,
            false,
        );
        let mut function = Function::new(
            local_types
                .into_iter()
                .map(|ty| (1, lowering.gc.val_type(ty))),
        );
        let locals = HashMap::new();
        let pattern_bindings = HashMap::new();
        let context = ExprContext {
            standard_library: lowering.standard_library,
            abi: lowering.abi,
            state: lowering.state,
            locals: LocalStorage::Wasm(&locals),
            globals: lowering.globals,
            global_types: lowering.global_types,
            settings: lowering.settings,
            runtime_globals: lowering.runtime_globals,
            runtime_helpers: lowering.runtime_helpers,
            functions: lowering.functions,
            equality_functions: lowering.equality_functions,
            records: lowering.records,
            enums: lowering.enums,
            arrays: lowering.arrays,
            memory: lowering.memory,
            abi_read: lowering.abi_read,
            matches: &matches,
            pattern_bindings: &pattern_bindings,
            semantics: lowering.semantics,
            wasm_ir: lowering.wasm_ir,
            gc: lowering.gc,
            loop_control: None,
        };
        compile_expr(&mut function, planned.expression, &context);
        function.instruction(&Instruction::End);
        return function;
    };
    let field_type_id = lowering
        .semantics
        .value_type(field.id)
        .expect("checked state fields have semantic types");
    let field_type = value_type(field.id, lowering.semantics);
    let field_size = lowering
        .memory
        .layout(field_type_id, lowering.semantics)
        .expect("checked pointer fields are MemoryReadable")
        .size();
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
    if let Some(provider) = lowering
        .state
        .provider
        .as_ref()
        .and_then(|provider| provider.resolved)
    {
        let provider = lowering.standard_library.state_provider(provider);
        let Implementation::Intrinsic(direct_read) = lowering
            .standard_library
            .item(provider.direct_read)
            .implementation;
        return match direct_read {
            IntrinsicId::GbaEmulatorRead => compile_gba_direct_read(
                path.offsets[0] as u32,
                field_type_id,
                field_type,
                field_size,
                result_type,
                lowering,
            ),
            _ => unreachable!("validated state-provider direct reads have backend lowering"),
        };
    }
    let mut function = Function::new([(1, ValType::I64)]);
    let address_local = 1;
    let offsets = &path.offsets;
    if let Some(module) = &path.module {
        let (ptr, len) = strings.get(module);
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
        emit_result_error(
            &mut function,
            result_type,
            field_type,
            "process module was not found",
            lowering.gc,
        );
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End)
            .instruction(&Instruction::LocalGet(address_local))
            .instruction(&Instruction::I64Const(offsets[0] as i64))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    } else {
        function
            .instruction(&Instruction::I64Const(offsets[0] as i64))
            .instruction(&Instruction::LocalSet(address_local));
    }

    let process_read = ProcessReadEmission {
        abi,
        address_local,
        fallback_ty: field_type,
        result_type,
        gc: lowering.gc,
        abi_read: lowering.abi_read,
    };
    for offset in offsets.iter().skip(1) {
        emit_process_read(&mut function, &process_read, 8);
        function
            .instruction(&Instruction::I32Const(lowering.abi_read.start()))
            .instruction(&Instruction::I64Load(memarg()))
            .instruction(&Instruction::I64Const(*offset as i64))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(address_local));
    }
    emit_process_read(&mut function, &process_read, field_size);
    emit_memory_value(
        &mut function,
        field_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
    );
    emit_result_success(&mut function, result_type, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

fn compile_gba_direct_read(
    address: u32,
    field_type_id: crate::types::TypeId,
    field_type: Type,
    field_size: u32,
    result_type: ResultTypeId,
    lowering: &EmissionContext<'_>,
) -> Function {
    let mut function = Function::new([(1, ValType::I64)]);
    let address_local = 1;
    function
        .instruction(&Instruction::GlobalGet(lowering.runtime_globals.process))
        .instruction(&Instruction::GlobalGet(
            lowering
                .runtime_globals
                .provider_value
                .expect("provider direct reads require provider storage"),
        ))
        .instruction(&Instruction::I32Const(address as i32))
        .instruction(&Instruction::I32Const(field_size as i32))
        .instruction(&Instruction::Call(
            lowering
                .runtime_helpers
                .function(RuntimeHelperId::GbaTranslateAddress),
        ))
        .instruction(&Instruction::LocalTee(address_local))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_result_error(
        &mut function,
        result_type,
        field_type,
        "invalid or unavailable GBA memory address",
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
            result_type,
            gc: lowering.gc,
            abi_read: lowering.abi_read,
        },
        field_size,
    );
    emit_memory_value(
        &mut function,
        field_type_id,
        lowering.abi_read,
        0,
        lowering.memory,
        lowering.semantics,
        lowering.gc,
    );
    emit_result_success(&mut function, result_type, lowering.gc);
    function.instruction(&Instruction::End);
    function
}

struct ProcessReadEmission<'a> {
    abi: &'a Abi,
    address_local: u32,
    fallback_ty: Type,
    result_type: ResultTypeId,
    gc: &'a GcLayout,
    abi_read: memory_plan::AbiReadScratch,
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
    emit_result_error(
        function,
        emission.result_type,
        emission.fallback_ty,
        "process read failed",
        emission.gc,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}

pub(super) fn compile_user_function(
    declaration: &FunctionDecl,
    lowering: &EmissionContext<'_>,
) -> Function {
    let wasm_body = lowering
        .wasm_ir
        .body(BodyOwner::Function(declaration.id))
        .expect("checked functions have Wasm IR bodies");
    let mut locals = declaration
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            (
                parameter.id,
                (index as u32, value_type(parameter.id, lowering.semantics)),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut local_types = Vec::new();
    let mut matches = MatchLayout::default();
    plan_wasm_locals(
        &wasm_body.locals,
        &mut locals,
        &mut matches,
        &mut local_types,
        declaration.params.len() as u32,
        lowering.semantics,
        true,
    );
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, lowering.gc.val_type(ty))),
    );
    let pattern_bindings = HashMap::new();
    let context = ExprContext {
        standard_library: lowering.standard_library,
        abi: lowering.abi,
        state: lowering.state,
        locals: LocalStorage::Wasm(&locals),
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        equality_functions: lowering.equality_functions,
        records: lowering.records,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        matches: &matches,
        pattern_bindings: &pattern_bindings,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        loop_control: None,
    };
    compile_block(&mut function, &wasm_body.entry, &context, None);
    if function_result(declaration.id, lowering.semantics) != Type::Void {
        function.instruction(&Instruction::Unreachable);
    }
    function.instruction(&Instruction::End);
    function
}

pub(super) fn compile_action(action: &Action, lowering: &EmissionContext<'_>) -> Function {
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
        2,
        lowering.semantics,
        true,
    );
    let mut function = Function::new(
        local_types
            .into_iter()
            .map(|ty| (1, lowering.gc.val_type(ty))),
    );
    let pattern_bindings = HashMap::new();
    let context = ExprContext {
        standard_library: lowering.standard_library,
        abi: lowering.abi,
        state: lowering.state,
        locals: LocalStorage::Wasm(&locals),
        globals: lowering.globals,
        global_types: lowering.global_types,
        settings: lowering.settings,
        runtime_globals: lowering.runtime_globals,
        runtime_helpers: lowering.runtime_helpers,
        functions: lowering.functions,
        equality_functions: lowering.equality_functions,
        records: lowering.records,
        enums: lowering.enums,
        arrays: lowering.arrays,
        memory: lowering.memory,
        abi_read: lowering.abi_read,
        matches: &matches,
        pattern_bindings: &pattern_bindings,
        semantics: lowering.semantics,
        wasm_ir: lowering.wasm_ir,
        gc: lowering.gc,
        loop_control: None,
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
        ActionKind::IsLoading => {
            function.instruction(&Instruction::I32Const(-1));
        }
        ActionKind::GameTime => {
            function.instruction(&Instruction::RefNull(HeapType::Concrete(
                gc.standard_index(StdlibTypeId::Duration),
            )));
        }
        ActionKind::OnDetached | ActionKind::OnAttach | ActionKind::WhileAttached => {}
    }
}

pub(super) fn plan_wasm_locals(
    planned: &[wasm_ir::Local],
    locals: &mut HashMap<ValueId, (u32, Type)>,
    matches: &mut MatchLayout,
    types: &mut Vec<Type>,
    parameter_count: u32,
    semantics: &SemanticModel,
    include_values: bool,
) {
    for local in planned {
        if matches!(local.purpose, LocalPurpose::Value(_)) && !include_values {
            continue;
        }
        let index = parameter_count + types.len() as u32;
        let ty = semantic_type(local.ty, semantics);
        types.push(ty);
        match local.purpose {
            LocalPurpose::Value(value) => {
                locals.insert(value, (index, ty));
            }
            LocalPurpose::MatchValue(expression) => {
                matches.values.insert(expression, index);
            }
            LocalPurpose::MatchBinding(pattern) => {
                matches.bindings.insert(pattern, (index, ty));
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
    context::EmissionContext,
    data_plan::StringPool,
    emit_memory_value, emit_result_error, emit_result_success,
    expression::{ExprContext, LocalStorage, MatchLayout, compile_block, compile_expr},
    function_result,
    imports::Abi,
    memarg, memory_plan, semantic_type, value_type,
};
