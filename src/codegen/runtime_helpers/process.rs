//! Process-memory, signature-scanning, and managed-string runtime helpers.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

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
