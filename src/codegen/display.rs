//! Lazy structural `Display` body generation.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    intrinsic_registry::RuntimeHelperId,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::StdlibTypeId,
    structural::{StructuralMemberId, StructuralType, StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, TypeId},
};

use super::{
    DisplayFunctions, GcLayout, RuntimeHelperPlan, Type, array_value, emit_string_literal,
    emit_typed_struct_get, enum_variant_payload, function_plan::UserFunctionPlan,
    record_field_type, try_array_element_type,
};

pub(super) struct DisplayInputs<'a> {
    pub structural: &'a StructuralTypes,
    pub arrays: &'a [ResolvedArrayType],
    pub semantics: &'a SemanticModel,
    pub displays: &'a DisplayFunctions,
    pub users: &'a HashMap<FunctionInstance, UserFunctionPlan>,
    pub helpers: &'a RuntimeHelperPlan,
    pub gc: &'a GcLayout,
}

pub(super) fn compile(inputs: &DisplayInputs<'_>) -> Vec<Function> {
    inputs
        .displays
        .derived
        .keys()
        .copied()
        .map(|ty| {
            let structural = inputs
                .structural
                .get(ty)
                .expect("derived Display implementations have structural metadata");
            match structural.id {
                StructuralTypeId::Record(_) => compile_record(structural, inputs),
                StructuralTypeId::Enum(_) => compile_enum(structural, inputs),
            }
        })
        .collect()
}

fn compile_record(record: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    let StructuralTypeId::Record(record_id) = record.id else {
        unreachable!()
    };
    let type_index = inputs.gc.index(Type::Record(record_id));
    emit_string_literal(&mut function, &format!("{} {{\n", record.name), inputs.gc);
    for (field_index, field) in record.members.iter().enumerate() {
        emit_string_literal(&mut function, &format!("    {}: ", field.name), inputs.gc);
        let StructuralMemberId::RecordField(field_id) = field.source else {
            unreachable!()
        };
        let field_type_id = field.ty.expect("record fields have semantic types");
        let field_type = record_field_type(field_id, inputs.semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_type);
        emit_value(&mut function, field_type_id, field_type, inputs);
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::IndentDisplay),
        ));
        emit_string_literal(&mut function, ",\n", inputs.gc);
    }
    emit_string_literal(&mut function, "}", inputs.gc);
    join_pieces(&mut function, 2 + record.members.len() as u32 * 3, inputs);
    function.instruction(&Instruction::End);
    function
}

fn compile_enum(enumeration: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let tag = 1;
    let StructuralTypeId::Enum(enum_id) = enumeration.id else {
        unreachable!()
    };
    let type_index = inputs.gc.index(Type::Enum(enum_id));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function.instruction(&Instruction::LocalSet(tag));

    for (variant_index, variant) in enumeration.members.iter().enumerate() {
        function
            .instruction(&Instruction::LocalGet(tag))
            .instruction(&Instruction::I32Const(variant_index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        if let Some(payload_type) = variant.ty {
            emit_string_literal(
                &mut function,
                &format!("{}.{}(\n    ", enumeration.name, variant.name),
                inputs.gc,
            );
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull);
            let StructuralMemberId::EnumVariant(variant_id) = variant.source else {
                unreachable!()
            };
            let payload = enum_variant_payload(variant_id, inputs.semantics)
                .expect("payload variants have backend types");
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, payload);
            emit_value(&mut function, payload_type, payload, inputs);
            function.instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::IndentDisplay),
            ));
            emit_string_literal(&mut function, ",\n)", inputs.gc);
            join_pieces(&mut function, 3, inputs);
        } else {
            emit_string_literal(
                &mut function,
                &format!("{}.{}", enumeration.name, variant.name),
                inputs.gc,
            );
        }
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    emit_string_literal(&mut function, &enumeration.name, inputs.gc);
    function.instruction(&Instruction::End);
    function
}

fn join_pieces(function: &mut Function, count: u32, inputs: &DisplayInputs<'_>) {
    let strings = inputs
        .arrays
        .iter()
        .find(|array| {
            try_array_element_type(array.id, inputs.semantics)
                == Some(Type::Standard(StdlibTypeId::String))
        })
        .expect("derived Display requires the runtime String array layout");
    array_value::emit_new_fixed(function, inputs.gc, strings.id, count);
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            inputs.gc.standard_index(StdlibTypeId::String),
        )))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::JoinStrings),
        ));
}

fn emit_value(function: &mut Function, ty: TypeId, backend: Type, inputs: &DisplayInputs<'_>) {
    if backend == Type::Standard(StdlibTypeId::String) {
        return;
    }
    if backend == Type::Bool {
        function.instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
        emit_string_literal(function, "true", inputs.gc);
        function.instruction(&Instruction::Else);
        emit_string_literal(function, "false", inputs.gc);
        function.instruction(&Instruction::End);
        return;
    }
    if backend == Type::Char {
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::FormatChar),
        ));
        return;
    }
    if let Some(display) = inputs.displays.custom.get(&ty) {
        function.instruction(&Instruction::Call(inputs.users[display].call));
        return;
    }
    if let Some(display) = inputs.displays.derived.get(&ty) {
        function.instruction(&Instruction::Call(*display));
        return;
    }
    emit_integer_to_i64(function, backend);
    function
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32Const(backend.is_signed() as i32))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::FormatI64),
        ));
}

fn emit_integer_to_i64(function: &mut Function, source: Type) {
    if matches!(source, Type::I8 | Type::I16 | Type::I32) {
        function.instruction(&Instruction::I64ExtendI32S);
    } else if matches!(source, Type::U8 | Type::U16 | Type::U32) {
        function.instruction(&Instruction::I64ExtendI32U);
    } else {
        debug_assert!(matches!(source, Type::I64 | Type::U64 | Type::Address));
    }
}
