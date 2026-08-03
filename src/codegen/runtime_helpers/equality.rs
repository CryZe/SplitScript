//! Structural equality body generation for runtime and source aggregates.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{
    ast::{EnumDecl, OptionTypeId, RecordDecl, ResultTypeId},
    intrinsic_registry::RuntimeHelperId,
    semantic::SemanticModel,
    stdlib::{DeclaredTypeRef, RuntimeRepresentation, StdlibTypeId},
    types::{ResolvedOptionType, ResolvedResultType},
};

use super::super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, Type, emit_typed_struct_get,
    enum_variant_payload, option_value_type, record_field_type, result_value_type,
    standard_field_type,
};
#[allow(clippy::too_many_arguments)]
pub(in crate::codegen) fn compile_equality(
    plan: &RuntimeHelperPlan,
    records: &[RecordDecl],
    enums: &[EnumDecl],
    options: &[ResolvedOptionType],
    results: &[ResolvedResultType],
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    gc: &GcLayout,
) -> Vec<Function> {
    let mut equality = Vec::new();
    // Payload-less/numeric-only structural equality never emits this call.
    // The sentinel is therefore only observed by the body builder when no
    // String edge exists, while dependency analysis requires the real helper
    // for every source-declared aggregate conservatively.
    let string_equality = plan
        .optional_function(RuntimeHelperId::StringEquality)
        .unwrap_or(0);
    for record in gc.standard_library.types() {
        if equality_functions.standard_records.contains_key(&record.id) {
            equality.push(compile_standard_record_equality(
                record.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for record in records {
        if equality_functions.records.contains_key(&record.id) {
            equality.push(compile_record_equality(
                record,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for enumeration in enums {
        if equality_functions.enums.contains_key(&enumeration.id) {
            equality.push(compile_enum_equality(
                enumeration,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for option in options {
        if equality_functions.options.contains_key(&option.id) {
            equality.push(compile_option_equality(
                option.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for result in results {
        if equality_functions.results.contains_key(&result.id) {
            equality.push(compile_result_equality(
                result.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }

    equality
}

pub(in crate::codegen::runtime_helpers) fn compile_string_eq(gc: &GcLayout) -> Function {
    let mut function = Function::new([(2, ValType::I32)]);
    let left = 0;
    let right = 1;
    let len = 2;
    let index = 3;

    function
        .instruction(&Instruction::LocalGet(left))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(len))
        .instruction(&Instruction::LocalGet(right))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(left))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalGet(right))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

fn compile_record_equality(
    record: &RecordDecl,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let type_index = gc.index(Type::Record(record.id));

    function.instruction(&Instruction::I32Const(1));
    for (field_index, field) in record.fields.iter().enumerate() {
        let ty = record_field_type(field.id, semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, ty);
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, ty);
        emit_value_equality(&mut function, ty, equality_functions, string_eq);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::End);
    function
}

fn compile_standard_record_equality(
    record: StdlibTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let ty = Type::Standard(record);
    let type_index = gc.index(ty);

    function.instruction(&Instruction::I32Const(1));
    for (field_index, field) in gc.standard_library.fields_of(record).enumerate() {
        let field_ty = standard_field_type(field.id, semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_ty);
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_ty);
        emit_value_equality(&mut function, field_ty, equality_functions, string_eq);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::End);
    function
}

fn compile_enum_equality(
    enumeration: &EnumDecl,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let tag = 2;
    let type_index = gc.index(Type::Enum(enumeration.id));

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function
        .instruction(&Instruction::LocalTee(tag))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    for (variant_index, variant) in enumeration.variants.iter().enumerate() {
        function
            .instruction(&Instruction::LocalGet(tag))
            .instruction(&Instruction::I32Const(variant_index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        if let Some(ty) = enum_variant_payload(variant.id, semantics) {
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, ty);
            function
                .instruction(&Instruction::LocalGet(1))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, ty);
            emit_value_equality(&mut function, ty, equality_functions, string_eq);
        } else {
            function.instruction(&Instruction::I32Const(1));
        }
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_option_equality(
    option: OptionTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let type_index = gc.index(Type::Option(option));
    let value_type = option_value_type(option, semantics);

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    emit_value_equality(&mut function, value_type, equality_functions, string_eq);
    function.instruction(&Instruction::End);
    function
}

fn compile_result_equality(
    result: ResultTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let tag = 2;
    let type_index = gc.index(Type::Result(result));
    let value_type = result_value_type(result, semantics);

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function
        .instruction(&Instruction::LocalTee(tag))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(tag))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    emit_value_equality(
        &mut function,
        Type::Standard(StdlibTypeId::String),
        equality_functions,
        string_eq,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    emit_value_equality(&mut function, value_type, equality_functions, string_eq);
    function.instruction(&Instruction::End);
    function
}

pub(in crate::codegen) fn emit_value_equality(
    function: &mut Function,
    ty: Type,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
) {
    let instruction = match ty {
        Type::Standard(StdlibTypeId::String) => Instruction::Call(string_eq),
        Type::Standard(standard) => match equality_functions
            .standard_library
            .type_decl(standard)
            .representation
        {
            RuntimeRepresentation::Scalar { storage } => {
                emit_value_equality(
                    function,
                    Type::from_declared(DeclaredTypeRef::Core(storage)),
                    equality_functions,
                    string_eq,
                );
                return;
            }
            RuntimeRepresentation::GcStruct { .. } => {
                Instruction::Call(equality_functions.standard_records[&standard])
            }
            RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                unreachable!("catalog validation rejected unsupported equality for `{standard:?}`")
            }
        },
        Type::Record(record) => Instruction::Call(equality_functions.records[&record]),
        Type::Enum(enumeration) => Instruction::Call(equality_functions.enums[&enumeration]),
        Type::Option(option) => Instruction::Call(equality_functions.options[&option]),
        Type::Result(result) => Instruction::Call(equality_functions.results[&result]),
        Type::None => Instruction::RefEq,
        Type::F32 => Instruction::F32Eq,
        Type::F64 => Instruction::F64Eq,
        Type::I64 | Type::U64 | Type::Address => Instruction::I64Eq,
        Type::Bool | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
            Instruction::I32Eq
        }
        _ => unreachable!("type checking rejected structural equality for `{ty:?}`"),
    };
    function.instruction(&instruction);
}
