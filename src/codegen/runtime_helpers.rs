//! Generated String, memory, signature-scan, Unity, and equality helpers.

use super::*;

pub(super) struct HelperBodies {
    pub core: Vec<Function>,
    pub equality: Vec<Function>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
    stdlib: &Stdlib,
    string_values: Option<&ArrayTypeDecl>,
    u64_offsets: Option<&ArrayTypeDecl>,
    records: &[RecordDecl],
    enums: &[EnumDecl],
    options: &[OptionTypeDecl],
    results: &[ResultTypeDecl],
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    dependencies: &BackendDependencies,
    gc: &GcLayout,
) -> HelperBodies {
    let mut core = Vec::new();
    for helper in dependencies.core_helpers() {
        let body = match helper {
            GeneratedHelper::PrintString => compile_print_string(abi),
            GeneratedHelper::TimerSetVariable => compile_timer_set_variable(abi),
            GeneratedHelper::FormatI64 => compile_format_i64(),
            GeneratedHelper::ConcatStrings => compile_concat_strings(
                gc.index(Type::Array(
                    string_values
                        .expect("String concatenation has a String array layout")
                        .id,
                )),
            ),
            GeneratedHelper::StringEquality => compile_string_eq(),
            GeneratedHelper::ScanProcessRange => compile_scan_process_range(abi),
            GeneratedHelper::ReadRelative32 => compile_read_relative32(abi),
            GeneratedHelper::ReadManagedString => compile_read_managed_string(abi),
            GeneratedHelper::FollowAddress => compile_follow_address(
                abi,
                gc.index(Type::Array(
                    u64_offsets
                        .expect("address following has a u64 array layout")
                        .id,
                )),
            ),
            GeneratedHelper::UnityAttach => compile_unity_attach(
                abi,
                strings,
                signatures,
                stdlib.helper(GeneratedHelper::ScanProcessRange),
                stdlib.helper(GeneratedHelper::ReadRelative32),
            ),
            GeneratedHelper::UnityGetImage => {
                compile_unity_get_image(abi, stdlib.helper(GeneratedHelper::CStringEquality))
            }
            GeneratedHelper::UnityGetClass => {
                compile_unity_get_class(abi, stdlib.helper(GeneratedHelper::CStringEquality))
            }
            GeneratedHelper::UnityGetFieldOffset => compile_unity_get_field_offset(
                abi,
                stdlib.helper(GeneratedHelper::CStringEquality),
                stdlib.helper(GeneratedHelper::BackingFieldEquality),
            ),
            GeneratedHelper::UnityGetFieldAny => compile_unity_get_field_any(
                stdlib.helper(GeneratedHelper::UnityGetFieldOffset),
                gc.index(Type::Array(
                    string_values
                        .expect("field alternatives have a String array layout")
                        .id,
                )),
            ),
            GeneratedHelper::UnityGetStaticInstance => compile_unity_get_static_instance(
                abi,
                stdlib.helper(GeneratedHelper::UnityGetFieldAny),
            ),
            GeneratedHelper::CStringEquality => compile_c_string_eq(abi),
            GeneratedHelper::BackingFieldEquality => compile_backing_field_eq(abi),
            GeneratedHelper::StringFromMemory | GeneratedHelper::RefreshSettings => {
                unreachable!("settings helpers are emitted by settings lowering")
            }
        };
        core.push(body);
    }

    let mut equality = Vec::new();
    // Payload-less/numeric-only structural equality never emits this call.
    // The sentinel is therefore only observed by the body builder when no
    // String edge exists, while dependency analysis requires the real helper
    // for every source-declared aggregate conservatively.
    let string_equality = stdlib
        .optional_helper(GeneratedHelper::StringEquality)
        .unwrap_or(0);
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

    HelperBodies { core, equality }
}

fn compile_print_string(abi: &Abi) -> Function {
    let mut function = Function::new([(3, ValType::I32)]);
    let string = 0;
    let len = 1;
    let index = 2;
    let required_pages = 3;

    function
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(len))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(65_535))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalTee(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::MemoryGrow(0))
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimePrintMessage),
        ))
        .instruction(&Instruction::End);
    function
}

fn compile_timer_set_variable(abi: &Abi) -> Function {
    let mut function = Function::new([(4, ValType::I32)]);
    let key = 0;
    let value = 1;
    let key_len = 2;
    let value_len = 3;
    let index = 4;
    let required_pages = 5;
    function
        .instruction(&Instruction::LocalGet(key))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(key_len))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(65_535))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::I32ShrU)
        .instruction(&Instruction::LocalTee(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(required_pages))
        .instruction(&Instruction::MemorySize(0))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::MemoryGrow(0))
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::End);
    emit_gc_string_copy_to_memory(&mut function, key, key_len, index, STRING_SCRATCH, None);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index));
    emit_gc_string_copy_to_memory(
        &mut function,
        value,
        value_len,
        index,
        STRING_SCRATCH,
        Some(key_len),
    );
    function
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Const(STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::TimerSetVariable),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_gc_string_copy_to_memory(
    function: &mut Function,
    string: u32,
    len: u32,
    index: u32,
    base: i32,
    additional_offset: Option<u32>,
) {
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(base));
    if let Some(offset) = additional_offset {
        function
            .instruction(&Instruction::LocalGet(offset))
            .instruction(&Instruction::I32Add);
    }
    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn compile_format_i64() -> Function {
    let mut function = Function::new([
        (2, ValType::I64),
        (3, ValType::I32),
        (1, val_type(Type::String)),
    ]);
    let input = 0;
    let signed = 1;
    let magnitude = 2;
    let remaining = 3;
    let digits = 4;
    let index = 5;
    let negative = 6;
    let output = 7;
    function
        .instruction(&Instruction::LocalGet(signed))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64LtS)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalSet(negative))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(input))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(magnitude))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(digits))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(digits))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(STRING_TYPE))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::LocalGet(digits))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(magnitude))
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64RemU)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::I32Const(b'0' as i32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArraySet(STRING_TYPE))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I64Const(10))
        .instruction(&Instruction::I64DivU)
        .instruction(&Instruction::LocalSet(remaining))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(negative))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(b'-' as i32))
        .instruction(&Instruction::ArraySet(STRING_TYPE))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn compile_concat_strings(strings_array: u32) -> Function {
    let mut function = Function::new([
        (5, ValType::I32),
        (1, val_type(Type::String)),
        (1, val_type(Type::String)),
    ]);
    let strings = 0;
    let string_index = 1;
    let total_len = 2;
    let byte_index = 3;
    let output_index = 4;
    let unused = 5;
    let current = 6;
    let output = 7;
    let _ = unused;
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(string_index));
    emit_array_get(&mut function, strings_array, Type::String);
    function
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalGet(total_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(total_len))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(total_len))
        .instruction(&Instruction::ArrayNewDefault(STRING_TYPE))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(strings))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(string_index));
    emit_array_get(&mut function, strings_array, Type::String);
    function
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(byte_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
        .instruction(&Instruction::ArraySet(STRING_TYPE))
        .instruction(&Instruction::LocalGet(byte_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_index))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(string_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(string_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn compile_string_eq() -> Function {
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
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
        .instruction(&Instruction::LocalGet(right))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
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
    emit_typed_struct_get(&mut function, type_index, 2, Type::String);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 2, Type::String);
    emit_value_equality(&mut function, Type::String, equality_functions, string_eq);
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

pub(super) fn emit_value_equality(
    function: &mut Function,
    ty: Type,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
) {
    let instruction = match ty {
        Type::String => Instruction::Call(string_eq),
        Type::Record(record) => Instruction::Call(equality_functions.records[&record]),
        Type::Enum(enumeration) => Instruction::Call(equality_functions.enums[&enumeration]),
        Type::Option(option) => Instruction::Call(equality_functions.options[&option]),
        Type::Result(result) => Instruction::Call(equality_functions.results[&result]),
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

/// Scans a process range in overlapping 4 KiB chunks. The overlap is one
/// pattern less than a page, so matches crossing a page boundary are retained.
/// Needle and mask bytes are compiler-produced static data.
fn compile_scan_process_range(abi: &Abi) -> Function {
    let mut function = Function::new([(2, ValType::I64), (5, ValType::I32)]);
    let process = 0;
    let address = 1;
    let size = 2;
    let needle = 3;
    let mask = 4;
    let len = 5;
    let offset = 6;
    let remaining = 7;
    let chunk = 8;
    let candidates = 9;
    let index = 10;
    let pattern_index = 11;
    let matched = 12;

    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::LocalTee(remaining))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64LtU)
        .instruction(&Instruction::BrIf(1))
        // chunk = min(remaining, 4096 + len - 1)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32Const(4095))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(4095))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(chunk))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(SCAN_SCRATCH))
        .instruction(&Instruction::LocalGet(chunk))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(4096))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::Br(1))
        .instruction(&Instruction::End)
        // candidates = min(remaining - len + 1, 4096)
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I64Const(4096))
        .instruction(&Instruction::I64LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(remaining))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(4096))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(candidates))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(candidates))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::LocalSet(matched))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(pattern_index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(pattern_index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(SCAN_SCRATCH))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(pattern_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::LocalGet(mask))
        .instruction(&Instruction::LocalGet(pattern_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::LocalGet(pattern_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(matched))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(pattern_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(pattern_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(offset))
        .instruction(&Instruction::I64Const(4096))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(offset))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_follow_address(abi: &Abi, offsets_array: u32) -> Function {
    let mut function = Function::new([(2, ValType::I32), (1, ValType::I64)]);
    let process = 0;
    let base = 1;
    let offsets = 2;
    let index = 3;
    let len = 4;
    let current = 5;

    function
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::LocalGet(offsets))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::LocalGet(offsets))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGet(offsets_array))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(current))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::End);
    function
}

fn compile_read_relative32(abi: &Abi) -> Function {
    let mut function = Function::new([]);
    let process = 0;
    let address = 1;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(4))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

fn compile_read_managed_string(abi: &Abi) -> Function {
    let mut function = Function::new([(7, ValType::I32), (1, val_type(Type::String))]);
    let process = 0;
    let address = 1;
    let max_units = 2;
    let units = 3;
    let input_index = 4;
    let byte_len = 5;
    let unit = 6;
    let low = 7;
    let codepoint = 8;
    let output_index = 9;
    let output = 10;

    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(0x10))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(units))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_empty_string_return(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(0x14))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(MANAGED_UTF16_SCRATCH))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_utf16_load(&mut function, input_index);
    function
        .instruction(&Instruction::LocalTee(unit))
        .instruction(&Instruction::I32Const(0xd800))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(unit))
        .instruction(&Instruction::I32Const(0xdbff))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    function
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_utf16_load_from_stack(&mut function);
    function
        .instruction(&Instruction::LocalTee(low))
        .instruction(&Instruction::I32Const(0xdc00))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(low))
        .instruction(&Instruction::I32Const(0xdfff))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::LocalGet(unit))
        .instruction(&Instruction::I32Const(0xd800))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::LocalGet(low))
        .instruction(&Instruction::I32Const(0xdc00))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(0x10000))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0xfffd))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0xfffd))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(unit))
        .instruction(&Instruction::I32Const(0xdc00))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(unit))
        .instruction(&Instruction::I32Const(0xdfff))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(0xfffd))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(unit))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(codepoint));

    emit_utf8_encode(&mut function, codepoint, byte_len);
    function
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::ArrayNewDefault(STRING_TYPE))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(MANAGED_UTF8_SCRATCH))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::ArraySet(STRING_TYPE))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn emit_empty_string_return(function: &mut Function) {
    function
        .instruction(&Instruction::ArrayNewFixed {
            array_type_index: STRING_TYPE,
            array_size: 0,
        })
        .instruction(&Instruction::Return);
}

fn emit_null_string_return(function: &mut Function) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(STRING_TYPE)))
        .instruction(&Instruction::Return);
}

fn emit_utf16_load(function: &mut Function, index: u32) {
    function.instruction(&Instruction::LocalGet(index));
    emit_utf16_load_from_stack(function);
}

fn emit_utf16_load_from_stack(function: &mut Function) {
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Const(MANAGED_UTF16_SCRATCH))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load16U(memarg()));
}

fn emit_utf8_store(function: &mut Function, byte_len: u32, value: impl FnOnce(&mut Function)) {
    function
        .instruction(&Instruction::I32Const(MANAGED_UTF8_SCRATCH))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Add);
    value(function);
    function.instruction(&Instruction::I32Store8(memarg()));
}

fn emit_utf8_encode(function: &mut Function, codepoint: u32, byte_len: u32) {
    function
        .instruction(&Instruction::LocalGet(codepoint))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_utf8_store(function, byte_len, |function| {
        function.instruction(&Instruction::LocalGet(codepoint));
    });
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(codepoint))
        .instruction(&Instruction::I32Const(0x800))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(6))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0xc0))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_len));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(0x3f))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Const(0x80))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(codepoint))
        .instruction(&Instruction::I32Const(0x10000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(12))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0xe0))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_len));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(6))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0x3f))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Const(0x80))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_len));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(0x3f))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Const(0x80))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else);
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(18))
            .instruction(&Instruction::I32ShrU)
            .instruction(&Instruction::I32Const(0xf0))
            .instruction(&Instruction::I32Or);
    });
    for shift in [12, 6] {
        function
            .instruction(&Instruction::LocalGet(byte_len))
            .instruction(&Instruction::I32Const(1))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::LocalSet(byte_len));
        emit_utf8_store(function, byte_len, |function| {
            function
                .instruction(&Instruction::LocalGet(codepoint))
                .instruction(&Instruction::I32Const(shift))
                .instruction(&Instruction::I32ShrU)
                .instruction(&Instruction::I32Const(0x3f))
                .instruction(&Instruction::I32And)
                .instruction(&Instruction::I32Const(0x80))
                .instruction(&Instruction::I32Or);
        });
    }
    function
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_len));
    emit_utf8_store(function, byte_len, |function| {
        function
            .instruction(&Instruction::LocalGet(codepoint))
            .instruction(&Instruction::I32Const(0x3f))
            .instruction(&Instruction::I32And)
            .instruction(&Instruction::I32Const(0x80))
            .instruction(&Instruction::I32Or);
    });
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(byte_len));
}

fn compile_unity_attach(
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
    scan_process_range: u32,
    read_relative32: u32,
) -> Function {
    let mut function = Function::new([(11, ValType::I64)]);
    let process = 0;
    let version = 1;
    let module_address = 2;
    let module_size = 3;
    let assemblies_match = 4;
    let assemblies = 5;
    let metadata = 6;
    let end = 7;
    let cursor = 8;
    let lea_match = 9;
    let lea = 10;
    let shr = 11;
    let type_info = 12;
    let (module_name, module_name_len) = strings.get("GameAssembly.dll");
    let assemblies_signature = signatures.get(IL2CPP_ASSEMBLIES_SIGNATURE);
    let metadata_signature = signatures.get(IL2CPP_METADATA_SIGNATURE);
    let lea_signature = signatures.get(IL2CPP_LEA_SIGNATURE);
    let shr_signature = signatures.get(IL2CPP_SHR_SIGNATURE);
    let rax_signature = signatures.get(IL2CPP_RAX_SIGNATURE);

    // Only layouts represented by ASR's IL2CPP version table are accepted.
    function
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Const(2019))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Const(2020))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Const(2022))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(module_name as i32))
        .instruction(&Instruction::I32Const(module_name_len as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleAddress),
        ))
        .instruction(&Instruction::LocalTee(module_address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(module_name as i32))
        .instruction(&Instruction::I32Const(module_name_len as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleSize),
        ))
        .instruction(&Instruction::LocalTee(module_size))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function.instruction(&Instruction::End);

    emit_static_scan_call(
        &mut function,
        process,
        module_address,
        module_size,
        assemblies_signature,
        scan_process_range,
    );
    function
        .instruction(&Instruction::LocalTee(assemblies_match))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assemblies_match))
        .instruction(&Instruction::I64Const(5))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalTee(assemblies))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function.instruction(&Instruction::End);

    emit_static_scan_call(
        &mut function,
        process,
        module_address,
        module_size,
        metadata_signature,
        scan_process_range,
    );
    function
        .instruction(&Instruction::LocalTee(metadata))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(module_address))
        .instruction(&Instruction::LocalGet(module_size))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(end))
        .instruction(&Instruction::LocalGet(module_address))
        .instruction(&Instruction::LocalSet(cursor))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I32Const(lea_signature.needle as i32))
        .instruction(&Instruction::I32Const(lea_signature.mask as i32))
        .instruction(&Instruction::I32Const(lea_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::LocalTee(lea_match))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalGet(metadata))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(lea))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(cursor))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(lea))
        .instruction(&Instruction::I64Const(0x200))
        .instruction(&Instruction::I32Const(shr_signature.needle as i32))
        .instruction(&Instruction::I32Const(shr_signature.mask as i32))
        .instruction(&Instruction::I32Const(shr_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::LocalTee(shr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(shr))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(shr))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(shr))
        .instruction(&Instruction::I64Const(0x100))
        .instruction(&Instruction::I32Const(rax_signature.needle as i32))
        .instruction(&Instruction::I32Const(rax_signature.mask as i32))
        .instruction(&Instruction::I32Const(rax_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(type_info))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(type_info))
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalTee(type_info))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(assemblies))
        .instruction(&Instruction::LocalGet(type_info))
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::StructNew(UNITY_MODULE_TYPE))
        .instruction(&Instruction::End);
    function
}

fn emit_static_scan_call(
    function: &mut Function,
    process: u32,
    address: u32,
    size: u32,
    signature: SignatureEntry,
    scan_process_range: u32,
) {
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(signature.needle as i32))
        .instruction(&Instruction::I32Const(signature.mask as i32))
        .instruction(&Instruction::I32Const(signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range));
}

fn emit_unity_attach_failure(function: &mut Function) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_MODULE_TYPE)))
        .instruction(&Instruction::Return);
}

fn compile_c_string_eq(abi: &Abi) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let process = 0;
    let address = 1;
    let expected = 2;
    let start = 3;
    let len = 4;
    let index = 5;
    function
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
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
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
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

/// Matches the C# compiler pattern `<Name>k__BackingField` without exposing
/// that metadata spelling to SplitScript programs.
fn compile_backing_field_eq(abi: &Abi) -> Function {
    let mut function = Function::new([(2, ValType::I32)]);
    let process = 0;
    let address = 1;
    let expected = 2;
    let expected_len = 3;
    let index = 4;
    function
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(expected_len))
        .instruction(&Instruction::I32Const(237))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Const(18))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH))
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(b'<' as i32))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH + 2))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::I64Const(i64::from_le_bytes(*b"k__Backi")))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH + 10))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::I64Const(i64::from_le_bytes(*b"ngField\0")))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(C_STRING_SCRATCH + 1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
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

fn compile_unity_get_image(abi: &Abi, c_string_eq: u32) -> Function {
    let mut function = Function::new([(6, ValType::I64)]);
    let process = 0;
    let module = 1;
    let expected_name = 2;
    let first = 3;
    let limit = 4;
    let index = 5;
    let assembly = 6;
    let name_ptr = 7;
    let image = 8;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_MODULE_TYPE,
            field_index: 0,
        })
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(16))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_IMAGE_TYPE)))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(first))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(limit))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(8))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(8))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(assembly))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assembly))
        .instruction(&Instruction::I64Const(0x18))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assembly))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(image))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(image))
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::StructNew(UNITY_IMAGE_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_IMAGE_TYPE)))
        .instruction(&Instruction::End);
    function
}

fn compile_unity_get_class(abi: &Abi, c_string_eq: u32) -> Function {
    let mut function = Function::new([(8, ValType::I64), (3, ValType::I32)]);
    let process = 0;
    let image_value = 1;
    let expected_name = 2;
    let metadata_ptr = 3;
    let type_info_table = 4;
    let classes = 5;
    let count = 6;
    let index = 7;
    let class = 8;
    let name_ptr = 9;
    let namespace_ptr = 10;
    let dot_plus_one = 11;
    let scan_index = 12;
    let metadata_handle = 13;

    // Find the last namespace separator in the requested class name.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::ArrayGetU(STRING_TYPE))
        .instruction(&Instruction::I32Const(b'.' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(dot_plus_one))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(scan_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    // Read the image's type count.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_IMAGE_TYPE,
            field_index: 0,
        })
        .instruction(&Instruction::I64Const(0x18))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalTee(count))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function.instruction(&Instruction::End);

    // V2020+ images point at a metadata handle, whose u32 selects the first
    // class pointer from the global type-info-definition table.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_IMAGE_TYPE,
            field_index: 0,
        })
        .instruction(&Instruction::I64Const(0x28))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(metadata_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(metadata_ptr))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalSet(metadata_handle))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_IMAGE_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_MODULE_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(type_info_table))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(type_info_table))
        .instruction(&Instruction::LocalGet(metadata_handle))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Const(8))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(classes))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(classes))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(8))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(class))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::I64Const(0x10))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::I64Const(0x18))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(namespace_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(namespace_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_IMAGE_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::StructNew(UNITY_CLASS_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_CLASS_TYPE)))
        .instruction(&Instruction::End);
    function
}

fn emit_unity_class_failure(function: &mut Function) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_CLASS_TYPE)))
        .instruction(&Instruction::Return);
}

/// Returns `field_offset + 1`, reserving zero for "not found yet" so a real
/// field at offset zero remains representable across an await retry.
fn compile_unity_get_field_offset(abi: &Abi, c_string_eq: u32, backing_field_eq: u32) -> Function {
    let mut function = Function::new([(9, ValType::I64)]);
    let process = 0;
    let class_value = 1;
    let expected_name = 2;
    let current = 3;
    let fields = 4;
    let count = 5;
    let index = 6;
    let field = 7;
    let name_ptr = 8;
    let parent = 9;
    let encoded = 10;
    let field_count_offset = 11;
    function
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_CLASS_TYPE,
            field_index: 0,
        })
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_CLASS_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_MODULE_TYPE,
            field_index: 2,
        })
        .instruction(&Instruction::I32Const(2022))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0x124))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_CLASS_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_MODULE_TYPE,
            field_index: 2,
        })
        .instruction(&Instruction::I32Const(2020))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0x120))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_CLASS_TYPE,
            field_index: 1,
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_MODULE_TYPE,
            field_index: 2,
        })
        .instruction(&Instruction::I32Const(2019))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0x11c))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0x114))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalSet(field_count_offset))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::LocalGet(field_count_offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load16U(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(count))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64Const(u16::MAX as i64))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(count))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(0x80))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(fields))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(fields))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(0x20))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(field))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::Call(backing_field_eq))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::I64Const(0x18))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(encoded))
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(0x58))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(parent))
        .instruction(&Instruction::LocalGet(parent))
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_unity_get_field_any(unity_get_field_offset: u32, names_array: u32) -> Function {
    let mut function = Function::new([(1, ValType::I32), (1, ValType::I64)]);
    let process = 0;
    let class_value = 1;
    let names = 2;
    let index = 3;
    let encoded = 4;
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index));
    emit_array_get(&mut function, names_array, Type::String);
    function
        .instruction(&Instruction::Call(unity_get_field_offset))
        .instruction(&Instruction::LocalTee(encoded))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::StructNew(UNITY_FIELD_TYPE))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::RefNull(HeapType::Concrete(UNITY_FIELD_TYPE)))
        .instruction(&Instruction::End);
    function
}

fn compile_unity_get_static_instance(abi: &Abi, unity_get_field_any: u32) -> Function {
    let mut function = Function::new([(1, val_type(Type::UnityField)), (1, ValType::I64)]);
    let process = 0;
    let class_value = 1;
    let names = 2;
    let field = 3;
    let static_table = 4;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::Call(unity_get_field_any))
        .instruction(&Instruction::LocalTee(field))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
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
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(static_table))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(static_table))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: UNITY_FIELD_TYPE,
            field_index: 0,
        })
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::End);
    function
}
