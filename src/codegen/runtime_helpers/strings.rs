//! String host-boundary, formatting, and concatenation runtime helpers.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

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

/// Compares a UTF-8 byte sequence using contains (0), starts with (1), ends with
/// (2), or equal while ignoring ASCII case (3) semantics. Exact byte matching
/// is equivalent to Unicode scalar matching for the first three modes because
/// both strings are valid UTF-8. The final mode folds only ASCII letters and is
/// deliberately not Unicode case folding.
pub(in crate::codegen::runtime_helpers) fn compile_string_match(gc: &GcLayout) -> Function {
    let mut function = Function::new([(7, ValType::I32)]);
    let value = 0;
    let needle = 1;
    let mode = 2;
    let value_len = 3;
    let needle_len = 4;
    let start = 5;
    let last_start = 6;
    let index = 7;
    let value_byte = 8;
    let needle_byte = 9;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(needle_len))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(last_start))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::LocalSet(start))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(value_byte))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalSet(needle_byte))
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(b'A' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(b'Z' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(value_byte))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(b'A' as i32))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(b'Z' as i32))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Const(32))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(needle_byte))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_byte))
        .instruction(&Instruction::LocalGet(needle_byte))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(mode))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(start))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

/// Finds the first exact UTF-8 byte match at or after `start`, returning `-1`
/// when no match exists. Both operands are valid UTF-8, so any non-empty match
/// starts and ends at code-point boundaries.
pub(in crate::codegen::runtime_helpers) fn compile_string_find(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([(5, ValType::I32)]);
    let value = 0;
    let needle = 1;
    let start = 2;
    let value_len = 3;
    let needle_len = 4;
    let candidate = 5;
    let last_start = 6;
    let index = 7;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(needle_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(last_start))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(needle_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::LocalGet(needle))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::LocalGet(last_start))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(candidate))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(candidate))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::End);
    function
}

/// Replaces exact, non-overlapping matches after first computing the precise
/// output size. A null reference is the failure sentinel for an empty search or
/// a result whose byte length cannot be represented by a WebAssembly array.
pub(in crate::codegen::runtime_helpers) fn compile_string_replace_all(
    string_find: u32,
    gc: &GcLayout,
) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (10, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let search = 1;
    let replacement = 2;
    let value_len = 3;
    let search_len = 4;
    let replacement_len = 5;
    let scan_index = 6;
    let match_index = 7;
    let match_count = 8;
    let length_delta = 9;
    let output_len = 10;
    let input_index = 11;
    let output_index = 12;
    let output = 13;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(search_len))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function, string_type);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(replacement_len))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(match_count))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(scan_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(length_delta))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::I32DivU)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function, string_type);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::LocalGet(length_delta))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(length_delta))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(match_count))
        .instruction(&Instruction::LocalGet(length_delta))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalSet(output_len))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::LocalGet(search))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::Call(string_find))
        .instruction(&Instruction::LocalTee(match_index))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: string_type,
            array_type_index_src: string_type,
        })
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::LocalGet(replacement_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(output_index))
        .instruction(&Instruction::LocalGet(match_index))
        .instruction(&Instruction::LocalGet(search_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

/// Extracts a UTF-8 byte range. Returning a null reference is the helper ABI's
/// failure sentinel for reversed or out-of-range bounds and offsets that split
/// a UTF-8 code point; expression lowering converts it to `String!`.
pub(in crate::codegen::runtime_helpers) fn compile_string_slice(gc: &GcLayout) -> Function {
    let string_type = gc.standard_index(StdlibTypeId::String);
    let mut function = Function::new([
        (3, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let value = 0;
    let start = 1;
    let end = 2;
    let value_len = 3;
    let output_len = 4;
    let copy_index = 5;
    let output = 6;

    function
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(value_len))
        // Reject reversed and out-of-range bounds.
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function, string_type);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // An offset at the end is a boundary. Every other boundary starts with
        // a byte whose two high bits are not `10`.
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function, string_type);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(value_len))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::I32Const(0xC0))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string(&mut function, string_type);
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::LocalTee(output_len))
        .instruction(&Instruction::ArrayNewDefault(string_type))
        .instruction(&Instruction::LocalSet(output))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(output_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::LocalGet(value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(string_type))
        .instruction(&Instruction::ArraySet(string_type))
        .instruction(&Instruction::LocalGet(copy_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(copy_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::End);
    function
}

fn emit_null_string(function: &mut Function, string_type: u32) {
    function.instruction(&Instruction::RefNull(HeapType::Concrete(string_type)));
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
