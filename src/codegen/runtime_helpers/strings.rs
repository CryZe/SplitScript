//! String host-boundary, formatting, and concatenation runtime helpers.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::imports::Abi;
use super::super::memory_plan::RuntimeScratch;
use super::super::{GcLayout, Type, emit_array_get, memarg};
pub(in crate::codegen::runtime_helpers) fn compile_print_string(
    abi: &Abi,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let host_strings = scratch.host_strings_start;
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
        .instruction(&Instruction::I32Const(host_strings))
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
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(string))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::RuntimePrintMessage),
        ))
        .instruction(&Instruction::End);
    function
}

pub(in crate::codegen::runtime_helpers) fn compile_timer_set_variable(
    abi: &Abi,
    gc: &GcLayout,
    scratch: RuntimeScratch,
) -> Function {
    let host_strings = scratch.host_strings_start;
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
        .instruction(&Instruction::I32Const(host_strings))
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
    emit_gc_string_copy_to_memory(&mut function, key, key_len, index, host_strings, None, gc);
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index));
    emit_gc_string_copy_to_memory(
        &mut function,
        value,
        value_len,
        index,
        host_strings,
        Some(key_len),
        gc,
    );
    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(key_len))
        .instruction(&Instruction::I32Const(host_strings))
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
    gc: &GcLayout,
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
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Store8(memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

pub(in crate::codegen::runtime_helpers) fn compile_format_i64(gc: &GcLayout) -> Function {
    let mut function = Function::new([
        (2, ValType::I64),
        (3, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
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
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
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
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
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
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

pub(in crate::codegen::runtime_helpers) fn compile_concat_strings(
    strings_array: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([
        (5, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
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
    emit_array_get(
        &mut function,
        strings_array,
        Type::Standard(StdlibTypeId::String),
    );
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
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
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
    emit_array_get(
        &mut function,
        strings_array,
        Type::Standard(StdlibTypeId::String),
    );
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
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
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
