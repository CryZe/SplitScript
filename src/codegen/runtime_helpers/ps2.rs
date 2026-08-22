//! PlayStation 2 hardware-address translation.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::GcLayout;
use super::super::imports::Abi;
use super::super::memory_plan::AbiReadScratch;

const BACKEND_POINTER_32: i32 = 2;
const BACKEND_POINTER_64: i32 = 3;
const BACKEND_RETROARCH: i32 = 4;

pub(super) fn compile_translate_address(
    abi: &Abi,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(1, ValType::I64), (1, ValType::I32)]);
    let emulator = 1;
    let address = 2;
    let size = 3;
    let base = 4;
    let backend = 5;
    let emulator_type = gc.standard_index(StdlibTypeId::PS2Emulator);

    function
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0010_0000))
        .instruction(&Instruction::I32LtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x0200_0000))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(0x0200_0000))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::PS2EmulatorBackend),
        })
        .instruction(&Instruction::LocalSet(backend))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::PS2EmulatorBase),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_POINTER_32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_process_read(&mut function, abi, abi_read, base, 4);
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
    emit_process_read(&mut function, abi, abi_read, base, 8);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(super::super::memarg()))
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_RETROARCH))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc
                .standard_field_index(crate::stdlib::StdlibFieldId::PS2EmulatorAuxiliary),
        })
        .instruction(&Instruction::LocalSet(base));
    emit_process_read(&mut function, abi, abi_read, base, 1);
    function
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::PS2EmulatorBase),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
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
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::End);
    function
}

fn emit_process_read(
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
