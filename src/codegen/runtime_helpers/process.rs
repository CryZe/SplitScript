//! Process-memory, signature-scanning, and managed-string runtime helpers.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{abi::AbiImportId, intrinsic_registry::MAX_NATIVE_STRING_BYTES, stdlib::StdlibTypeId};

use super::super::imports::Abi;
use super::super::memory_plan::{AbiReadScratch, ScratchRegion};
use super::super::{GcLayout, Type, memarg};
pub(super) fn compile_scan_process_range(abi: &Abi, scan: ScratchRegion) -> Function {
    let scan_start = scan.start();
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
        .instruction(&Instruction::I32Const(scan_start))
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
        .instruction(&Instruction::I32Const(scan_start))
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

pub(super) fn compile_follow_address(
    abi: &Abi,
    offsets_array: u32,
    abi_read: AbiReadScratch,
) -> Function {
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
        .instruction(&Instruction::I32Const(abi_read.destination(8)))
        .instruction(&Instruction::I32Const(8))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
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

pub(super) fn compile_read_relative32(abi: &Abi, abi_read: AbiReadScratch) -> Function {
    let mut function = Function::new([]);
    let process = 0;
    let address = 1;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(abi_read.destination(4)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(4))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_read_utf8_string(
    abi: &Abi,
    string_from_memory: u32,
    gc: &GcLayout,
    native_utf8: ScratchRegion,
) -> Function {
    let native_utf8_start = native_utf8.destination(MAX_NATIVE_STRING_BYTES);
    let mut function = Function::new([(5, ValType::I32)]);
    let process = 0;
    let address = 1;
    let max_bytes = 2;
    let byte_len = 3;
    let index = 4;
    let byte = 5;
    let width = 6;
    let next = 7;

    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(max_bytes))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(max_bytes))
        .instruction(&Instruction::I32Const(MAX_NATIVE_STRING_BYTES as i32))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(native_utf8_start))
        .instruction(&Instruction::LocalGet(max_bytes))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    // Find the first NUL byte. If none occurs within the bound, the complete
    // bounded region is the string payload, matching ASR's ArrayCString.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(max_bytes))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_scratch_byte(&mut function, native_utf8_start, index, 0);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(byte_len))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index));

    // Validate strictly before constructing a SplitScript String. The string
    // representation contains UTF-8 bytes, so malformed process data is a
    // Result failure rather than an invalid language value.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_scratch_byte(&mut function, native_utf8_start, index, 0);
    function
        .instruction(&Instruction::LocalTee(byte))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xc2))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xdf))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xe0))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xef))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(3))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xf0))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xf4))
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalTee(width))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32GtU);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_scratch_byte(&mut function, native_utf8_start, index, 1);
    function.instruction(&Instruction::LocalSet(next));
    emit_invalid_continuation(&mut function, next);
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xe0))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(next))
        .instruction(&Instruction::I32Const(0xa0))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xed))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(next))
        .instruction(&Instruction::I32Const(0x9f))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xf0))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(next))
        .instruction(&Instruction::I32Const(0x90))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0xf4))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::LocalGet(next))
        .instruction(&Instruction::I32Const(0x8f))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);
    function.instruction(&Instruction::End);

    for (required_width, offset) in [(3, 2), (4, 3)] {
        function
            .instruction(&Instruction::LocalGet(width))
            .instruction(&Instruction::I32Const(required_width))
            .instruction(&Instruction::I32GeU)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_scratch_byte(&mut function, native_utf8_start, index, offset);
        function.instruction(&Instruction::LocalSet(next));
        emit_invalid_continuation(&mut function, next);
        emit_null_string_if(&mut function, gc);
        function.instruction(&Instruction::End);
    }

    function
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(width))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(native_utf8_start))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_read_utf16_le_string(
    abi: &Abi,
    utf16_from_memory: u32,
    gc: &GcLayout,
    utf16: ScratchRegion,
) -> Function {
    let utf16_start = utf16.destination(
        crate::intrinsic_registry::MAX_NATIVE_UTF16_UNITS
            .checked_mul(2)
            .expect("bounded UTF-16 input must fit wasm32"),
    );
    let mut function = Function::new([(2, ValType::I32)]);
    let process = 0;
    let address = 1;
    let max_units = 2;
    let units = 3;
    let index = 4;

    function
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::I32Or)
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32Const(
            crate::intrinsic_registry::MAX_NATIVE_UTF16_UNITS as i32,
        ))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(utf16_start))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    // Native strings terminate at the first complete NUL code unit. If the
    // bounded region has no terminator, decode the full region.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_utf16_load(&mut function, index, utf16_start);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(units))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::Call(utf16_from_memory))
        .instruction(&Instruction::End);
    function
}

/// Decodes `units` UTF-16LE code units from the shared input scratch region.
/// Malformed surrogate sequences use Unicode replacement semantics, matching
/// Rust's `decode_utf16` and .NET's replacement decoder behavior.
pub(super) fn compile_utf16_string_from_memory(
    gc: &GcLayout,
    utf16: ScratchRegion,
    utf8: ScratchRegion,
) -> Function {
    let utf16_start = utf16.start();
    let utf8_start = utf8.start();
    let mut function = Function::new([
        (6, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
    let units = 0;
    let input_index = 1;
    let byte_len = 2;
    let unit = 3;
    let low = 4;
    let codepoint = 5;
    let output_index = 6;
    let output = 7;

    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_utf16_load(&mut function, input_index, utf16_start);
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
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_utf16_load_from_stack(&mut function, utf16_start);
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

    emit_utf8_encode(&mut function, codepoint, byte_len, utf8_start);
    function
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
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
        .instruction(&Instruction::I32Const(utf8_start))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
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

pub(super) fn compile_read_managed_string(
    abi: &Abi,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
    utf16: ScratchRegion,
    utf8: ScratchRegion,
) -> Function {
    let utf16_start = utf16.start();
    let utf8_start = utf8.start();
    let mut function = Function::new([
        (7, ValType::I32),
        (1, gc.val_type(Type::Standard(StdlibTypeId::String))),
    ]);
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
    emit_null_string_return(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(0x10))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(4)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalGet(max_units))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(abi_read.start()))
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
    emit_empty_string_return(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(0x14))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(utf16_start))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::LocalGet(units))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1));
    emit_utf16_load(&mut function, input_index, utf16_start);
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
    emit_utf16_load_from_stack(&mut function, utf16_start);
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

    emit_utf8_encode(&mut function, codepoint, byte_len, utf8_start);
    function
        .instruction(&Instruction::LocalGet(input_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(input_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::ArrayNewDefault(
            gc.standard_index(StdlibTypeId::String),
        ))
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
        .instruction(&Instruction::I32Const(utf8_start))
        .instruction(&Instruction::LocalGet(output_index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::ArraySet(
            gc.standard_index(StdlibTypeId::String),
        ))
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

pub(super) fn compile_module_path(
    abi: &Abi,
    string_from_memory: u32,
    gc: &GcLayout,
    scratch: super::super::memory_plan::RuntimeScratch,
) -> Function {
    const MAX_MODULE_PATH_BYTES: i32 = 65_536;

    let host_strings = scratch.host_strings_start;
    let path_length_pointer = scratch.abi_read.destination(4);
    let mut function = Function::new([(4, ValType::I32)]);
    let process = 0;
    let name = 1;
    let name_length = 2;
    let index = 3;
    let path_length = 4;
    let required_pages = 5;

    function
        .instruction(&Instruction::LocalGet(name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalSet(name_length))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(MAX_MODULE_PATH_BYTES))
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
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(name))
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
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModulePath),
        ))
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalTee(path_length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::I32Const(MAX_MODULE_PATH_BYTES))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModulePath),
        ))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(name_length))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_process_path(
    abi: &Abi,
    string_from_memory: u32,
    gc: &GcLayout,
    scratch: super::super::memory_plan::RuntimeScratch,
) -> Function {
    const MAX_PROCESS_PATH_BYTES: i32 = 65_536;

    let host_strings = scratch.host_strings_start;
    let path_length_pointer = scratch.abi_read.destination(4);
    let mut function = Function::new([(2, ValType::I32)]);
    let process = 0;
    let path_length = 1;
    let required_pages = 2;

    emit_ensure_linear_capacity(
        &mut function,
        host_strings + MAX_PROCESS_PATH_BYTES,
        required_pages,
    );
    function
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetPath),
        ))
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalTee(path_length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::I32Const(MAX_PROCESS_PATH_BYTES))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::I32Const(path_length_pointer))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetPath),
        ))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(path_length))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_runtime_metadata(
    abi: &Abi,
    import: AbiImportId,
    string_from_memory: u32,
    gc: &GcLayout,
    scratch: super::super::memory_plan::RuntimeScratch,
) -> Function {
    const MAX_RUNTIME_METADATA_BYTES: i32 = 256;

    debug_assert!(matches!(
        import,
        AbiImportId::RuntimeGetOs | AbiImportId::RuntimeGetArch
    ));
    let host_strings = scratch.host_strings_start;
    let length_pointer = scratch.abi_read.destination(4);
    let mut function = Function::new([(2, ValType::I32)]);
    let length = 0;
    let required_pages = 1;

    emit_ensure_linear_capacity(
        &mut function,
        host_strings + MAX_RUNTIME_METADATA_BYTES,
        required_pages,
    );
    function
        .instruction(&Instruction::I32Const(length_pointer))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Store(memarg()))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(length_pointer))
        .instruction(&Instruction::Call(abi.function(import)))
        .instruction(&Instruction::Drop)
        .instruction(&Instruction::I32Const(length_pointer))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalTee(length))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(MAX_RUNTIME_METADATA_BYTES))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::I32Const(length_pointer))
        .instruction(&Instruction::Call(abi.function(import)))
        .instruction(&Instruction::I32Eqz);
    emit_null_string_if(&mut function, gc);

    function
        .instruction(&Instruction::I32Const(host_strings))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::Call(string_from_memory))
        .instruction(&Instruction::End);
    function
}

fn emit_ensure_linear_capacity(function: &mut Function, end: i32, required_pages: u32) {
    function
        .instruction(&Instruction::I32Const(end))
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
}

fn emit_empty_string_return(function: &mut Function, gc: &GcLayout) {
    function
        .instruction(&Instruction::ArrayNewFixed {
            array_type_index: gc.standard_index(StdlibTypeId::String),
            array_size: 0,
        })
        .instruction(&Instruction::Return);
}

fn emit_null_string_return(function: &mut Function, gc: &GcLayout) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::String),
        )))
        .instruction(&Instruction::Return);
}

fn emit_null_string_if(function: &mut Function, gc: &GcLayout) {
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_null_string_return(function, gc);
    function.instruction(&Instruction::End);
}

fn emit_scratch_byte(function: &mut Function, start: i32, index: u32, offset: i32) {
    function
        .instruction(&Instruction::I32Const(start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(wasm_encoder::MemArg {
            offset: offset as u64,
            ..memarg()
        }));
}

/// Leaves true on the operand stack when `local` is not a UTF-8 continuation
/// byte. The caller chooses the surrounding failure boundary.
fn emit_invalid_continuation(function: &mut Function, local: u32) {
    function
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::LocalGet(local))
        .instruction(&Instruction::I32Const(0xbf))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::I32Or);
}

fn emit_utf16_load(function: &mut Function, index: u32, utf16_start: i32) {
    function.instruction(&Instruction::LocalGet(index));
    emit_utf16_load_from_stack(function, utf16_start);
}

fn emit_utf16_load_from_stack(function: &mut Function, utf16_start: i32) {
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Shl)
        .instruction(&Instruction::I32Const(utf16_start))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load16U(memarg()));
}

fn emit_utf8_store(
    function: &mut Function,
    byte_len: u32,
    utf8_start: i32,
    value: impl FnOnce(&mut Function),
) {
    function
        .instruction(&Instruction::I32Const(utf8_start))
        .instruction(&Instruction::LocalGet(byte_len))
        .instruction(&Instruction::I32Add);
    value(function);
    function.instruction(&Instruction::I32Store8(memarg()));
}

fn emit_utf8_encode(function: &mut Function, codepoint: u32, byte_len: u32, utf8_start: i32) {
    function
        .instruction(&Instruction::LocalGet(codepoint))
        .instruction(&Instruction::I32Const(0x80))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_utf8_store(function, byte_len, utf8_start, |function| {
        function.instruction(&Instruction::LocalGet(codepoint));
    });
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(codepoint))
        .instruction(&Instruction::I32Const(0x800))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
        emit_utf8_store(function, byte_len, utf8_start, |function| {
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
    emit_utf8_store(function, byte_len, utf8_start, |function| {
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
