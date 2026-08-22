//! Game Boy Advance hardware-address translation.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::GcLayout;
use super::super::imports::Abi;
use super::super::memory_plan::AbiReadScratch;

const BACKEND_POINTER_32: i32 = 2;
const BACKEND_POINTER_64: i32 = 3;
const BACKEND_NOCASH: i32 = 4;

pub(super) fn compile_translate_address(
    abi: &Abi,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(2, ValType::I64), (1, ValType::I32)]);
    let emulator = 1;
    let address = 2;
    let size = 3;
    let base = 4;
    let pointer = 5;
    let backend = 6;
    let emulator_type = gc.standard_index(StdlibTypeId::GBAEmulator);

    function
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // Ensure the selected backend is initialized.
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorBackend),
        })
        .instruction(&Instruction::LocalTee(backend))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        // EWRAM: 0x02000000..0x02040000.
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0200_0000))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0204_0000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(0x0204_0000))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorEwram),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorAux1),
        })
        .instruction(&Instruction::LocalSet(pointer))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        // IWRAM: 0x03000000..0x03008000.
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0300_0000))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0300_8000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(0x0300_8000))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32LeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorIwram),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorAux2),
        })
        .instruction(&Instruction::LocalSet(pointer))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        // Pointer-backed emulators can move RAM when emulation starts or a
        // ROM is reloaded. Resolve their current base for every script read.
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_POINTER_32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_translate_process_read(&mut function, abi, abi_read, pointer, 4);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_POINTER_64))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_translate_process_read(&mut function, abi, abi_read, pointer, 8);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(super::super::memarg()))
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_NOCASH))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GBAEmulatorAux1),
        })
        .instruction(&Instruction::LocalSet(pointer));
    emit_translate_process_read(&mut function, abi, abi_read, pointer, 4);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0300_0000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(0x9394))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0x95d4))
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(pointer));
    emit_translate_process_read(&mut function, abi, abi_read, pointer, 4);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0300_0000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0200_0000))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0300_0000))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::End);
    function
}

fn emit_translate_process_read(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    size: u32,
) {
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);
}
