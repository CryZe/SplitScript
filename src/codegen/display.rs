//! Lazy structural `Debug` body generation used by `Display` fallback.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{
    ast::RangeKind,
    intrinsic_registry::RuntimeHelperId,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{StdlibTypeConstructorId, StdlibTypeId},
    structural::{StructuralMemberId, StructuralType, StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, TypeId, TypeKind},
};

use super::{
    DisplayFunctions, GcLayout, RuntimeHelperPlan, Type, array_value, emit_array_get,
    emit_string_literal, emit_typed_struct_get, enum_variant_payload,
    function_plan::UserFunctionPlan, record_field_type, try_array_element_type,
};

pub(super) struct DisplayInputs<'a> {
    pub structural: &'a StructuralTypes,
    pub arrays: &'a [ResolvedArrayType],
    pub semantics: &'a SemanticModel,
    pub displays: &'a DisplayFunctions,
    pub users: &'a HashMap<FunctionInstance, UserFunctionPlan>,
    pub helpers: &'a RuntimeHelperPlan,
    pub debug_depth: u32,
    pub gc: &'a GcLayout,
}

pub(super) fn compile(inputs: &DisplayInputs<'_>) -> Vec<Function> {
    inputs
        .displays
        .derived
        .keys()
        .copied()
        .map(|ty| {
            if let Some(structural) = inputs.structural.get(ty) {
                return match structural.id {
                    StructuralTypeId::Record(_) => compile_record(structural, inputs),
                    StructuralTypeId::Enum(_) => compile_enum(structural, inputs),
                };
            }
            match inputs.semantics.types().kind(ty) {
                TypeKind::Array {
                    layout, element, ..
                } => compile_array(*layout, *element, inputs),
                TypeKind::Set {
                    layout,
                    element,
                    backing,
                } => compile_set(*layout, *element, *backing, inputs),
                TypeKind::Option { layout, value } => compile_option(*layout, *value, inputs),
                TypeKind::Result { layout, value } => compile_result(*layout, *value, inputs),
                TypeKind::Range {
                    layout,
                    bound,
                    kind,
                } => compile_range(*layout, *bound, *kind, inputs),
                TypeKind::Application {
                    layout,
                    constructor: StdlibTypeConstructorId::IteratorStep,
                    arguments,
                } => compile_iterator_step(*layout, arguments[0], inputs),
                kind => unreachable!("derived Debug implementation for {kind:?}"),
            }
        })
        .collect()
}

fn compile_record(record: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
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
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_enum(enumeration: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    begin_recursion_guard(&mut function, inputs);
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
        decrement_debug_depth(&mut function, inputs);
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    emit_string_literal(&mut function, &enumeration.name, inputs.gc);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_array(
    array: crate::ast::ArrayTypeId,
    element: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let storage = array_value::storage_id(array, inputs.arrays, inputs.semantics);
    compile_sequence(
        "[\n",
        "]",
        element,
        Type::ArrayStorage(storage),
        |function| {
            function.instruction(&Instruction::LocalGet(0));
            array_value::emit_length(function, inputs.gc, array);
        },
        |function| {
            function.instruction(&Instruction::LocalGet(0));
            array_value::emit_backing(function, inputs.gc, array);
        },
        inputs,
    )
}

fn compile_set(
    set: crate::ast::TypeApplicationId,
    element: TypeId,
    backing: crate::ast::ArrayTypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    compile_sequence(
        "Set {\n",
        "}",
        element,
        Type::ArrayStorage(backing),
        |function| {
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: inputs.gc.index(Type::Set(set)),
                    field_index: super::set_functions::LENGTH_FIELD,
                });
        },
        |function| {
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull)
                .instruction(&Instruction::StructGet {
                    struct_type_index: inputs.gc.index(Type::Set(set)),
                    field_index: super::set_functions::BACKING_FIELD,
                })
                .instruction(&Instruction::RefAsNonNull);
        },
        inputs,
    )
}

fn compile_sequence(
    opening: &str,
    closing: &str,
    element: TypeId,
    source_storage: Type,
    emit_length: impl Fn(&mut Function),
    emit_backing: impl Fn(&mut Function),
    inputs: &DisplayInputs<'_>,
) -> Function {
    let (strings, string_storage) = string_array(inputs);
    let mut function = Function::new([
        (2, ValType::I32),
        (1, inputs.gc.val_type(Type::ArrayStorage(string_storage))),
    ]);
    let index = 1;
    let length = 2;
    let pieces = 3;
    begin_recursion_guard(&mut function, inputs);
    emit_length(&mut function);
    function
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalSet(pieces));
    set_piece_literal(&mut function, pieces, 0, opening, inputs);
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_backing(&mut function);
    function.instruction(&Instruction::LocalGet(index));
    let backend = super::semantic_type(element, inputs.semantics);
    emit_array_get(
        &mut function,
        inputs.gc.index(source_storage),
        backend,
        inputs.gc,
    );
    emit_value(&mut function, element, backend, inputs);
    function
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::WrapDebugEntry),
        ))
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_string_literal(&mut function, closing, inputs.gc);
    function
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add);
    array_value::emit_wrap_loaded(&mut function, inputs.gc.index(Type::Array(strings)));
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            inputs.gc.standard_index(StdlibTypeId::String),
        )))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::JoinStrings),
        ));
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_option(
    option: crate::ast::OptionTypeId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(&mut function, "None", inputs.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(
        &mut function,
        inputs.gc.index(Type::Option(option)),
        0,
        backend,
    );
    emit_unary("Some", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_result(
    result: crate::ast::ResultTypeId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let type_index = inputs.gc.index(Type::Result(result));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function.instruction(&Instruction::If(BlockType::Result(
        inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
    )));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    let string = inputs
        .semantics
        .types()
        .id_for_standard(StdlibTypeId::String);
    emit_unary(
        "Err",
        string,
        Type::Standard(StdlibTypeId::String),
        &mut function,
        inputs,
    );
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(&mut function, type_index, 0, backend);
    emit_unary("Ok", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_range(
    range: crate::ast::RangeTypeId,
    bound: TypeId,
    kind: RangeKind,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let backend = super::semantic_type(bound, inputs.semantics);
    let type_index = inputs.gc.index(Type::Range(range));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, backend);
    emit_value(&mut function, bound, backend, inputs);
    emit_string_literal(
        &mut function,
        match kind {
            RangeKind::Exclusive => "..<",
            RangeKind::Inclusive => "..=",
        },
        inputs.gc,
    );
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, backend);
    emit_value(&mut function, bound, backend, inputs);
    join_pieces(&mut function, 3, inputs);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_iterator_step(
    step: crate::ast::TypeApplicationId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(&mut function, "End", inputs.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(
        &mut function,
        inputs.gc.index(Type::Application(step)),
        0,
        backend,
    );
    emit_unary("Item", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn begin_recursion_guard(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(64))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(function, "<cycle>", inputs.gc);
    function.instruction(&Instruction::Else);
    increment_debug_depth(function, inputs);
}

fn increment_debug_depth(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::GlobalSet(inputs.debug_depth));
}

fn decrement_debug_depth(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::GlobalSet(inputs.debug_depth));
}

fn finish_recursion_guard(function: &mut Function, inputs: &DisplayInputs<'_>) {
    // The formatted String stays below the depth update on the operand stack.
    decrement_debug_depth(function, inputs);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_unary(
    name: &str,
    value: TypeId,
    backend: Type,
    function: &mut Function,
    inputs: &DisplayInputs<'_>,
) {
    emit_value(function, value, backend, inputs);
    emit_string_literal(function, name, inputs.gc);
    function.instruction(&Instruction::Call(
        inputs.helpers.function(RuntimeHelperId::WrapDebugVariant),
    ));
}

fn string_array(inputs: &DisplayInputs<'_>) -> (crate::ast::ArrayTypeId, crate::ast::ArrayTypeId) {
    let array = inputs
        .arrays
        .iter()
        .find(|array| {
            try_array_element_type(array.id, inputs.semantics)
                == Some(Type::Standard(StdlibTypeId::String))
        })
        .expect("derived Debug requires the runtime String array layout")
        .id;
    let storage = array_value::storage_id(array, inputs.arrays, inputs.semantics);
    (array, storage)
}

fn set_piece_literal(
    function: &mut Function,
    pieces: u32,
    index: u32,
    value: &str,
    inputs: &DisplayInputs<'_>,
) {
    let (_, storage) = string_array(inputs);
    function
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(index as i32));
    emit_string_literal(function, value, inputs.gc);
    function.instruction(&Instruction::ArraySet(
        inputs.gc.index(Type::ArrayStorage(storage)),
    ));
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
        function
            .instruction(&Instruction::I32Const(b'"' as i32))
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::QuoteDebugString),
            ));
        return;
    }
    if backend == Type::None {
        function.instruction(&Instruction::Drop);
        emit_string_literal(function, "None", inputs.gc);
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
        function
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::FormatChar),
            ))
            .instruction(&Instruction::I32Const(b'\'' as i32))
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::QuoteDebugString),
            ));
        return;
    }
    if let Some(debug) = inputs.displays.custom_debug.get(&ty) {
        function.instruction(&Instruction::Call(inputs.users[debug].call));
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
