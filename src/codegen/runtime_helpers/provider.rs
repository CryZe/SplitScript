//! Shared normalized-read adapter for address-translating emulator providers.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::abi::AbiImportId;

use super::super::imports::Abi;

/// Translates one guest range and fills the caller-provided scratch range.
///
/// Status 0 means the guest address is invalid or unavailable, 1 means the
/// requested bytes were written, and 2 means the translated host read failed.
/// Keeping byte acquisition behind this contract lets providers such as
/// Genesis normalize non-contiguous or word-swapped storage without changing
/// the shared type-directed decoder.
pub(super) fn compile_translated_read(abi: &Abi, translate_address: u32) -> Function {
    let mut function = Function::new([(1, ValType::I64)]);
    let native_address = 5;

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::LocalGet(2))
        .instruction(&Instruction::LocalGet(4))
        .instruction(&Instruction::Call(translate_address))
        .instruction(&Instruction::LocalTee(native_address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(native_address))
        .instruction(&Instruction::LocalGet(3))
        .instruction(&Instruction::LocalGet(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}
