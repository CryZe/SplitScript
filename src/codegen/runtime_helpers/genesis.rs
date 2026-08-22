//! Sega Genesis guest-byte acquisition.
//!
//! Several supported emulators expose work RAM with the two bytes in every
//! native 16-bit word reversed. This helper deliberately normalizes that
//! storage before the shared big-endian `MemoryReadable` decoder runs. Keeping
//! the quirk here also makes unaligned scalar, record, array, and guest-pointer
//! reads agree with one another.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::GcLayout;
use super::super::imports::Abi;

const BACKEND_STABLE: i32 = 1;
const BACKEND_POINTER_32: i32 = 2;
const BACKEND_RETROARCH: i32 = 4;

/// Fills the caller-provided destination with normalized guest bytes.
///
/// Status 0 means the guest range or retained mapping is unavailable, 1 means
/// the destination was filled, and 2 means the final native memory read failed.
pub(super) fn compile_read_memory(abi: &Abi, gc: &GcLayout) -> Function {
    // Parameters: process, provider, guest address, destination, byte count.
    let mut function = Function::new([(1, ValType::I64), (6, ValType::I32)]);
    let process = 0;
    let emulator = 1;
    let address = 2;
    let destination = 3;
    let size = 4;
    let base = 5;
    let backend = 6;
    let word_swapped = 7;
    let aligned_address = 8;
    let read_size = 9;
    let index = 10;
    let temporary_byte = 11;
    let emulator_type = gc.standard_index(StdlibTypeId::GenesisEmulator);

    function
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(0x1_0000))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(0x1_0000))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc
                .standard_field_index(crate::stdlib::StdlibFieldId::GenesisEmulatorBackend),
        })
        .instruction(&Instruction::LocalSet(backend))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GenesisEmulatorBase),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGetU {
            struct_type_index: emulator_type,
            field_index: gc
                .standard_field_index(crate::stdlib::StdlibFieldId::GenesisEmulatorWordSwapped),
        })
        .instruction(&Instruction::LocalSet(word_swapped));

    // Libretro keeps a stable RAM base, but the core itself may unload while
    // the host process remains alive. Touch the retained module base first.
    function
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_RETROARCH))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc
                .standard_field_index(crate::stdlib::StdlibFieldId::GenesisEmulatorAuxiliary),
        })
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_POINTER_32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I32Const(BACKEND_STABLE))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 0);
    function.instruction(&Instruction::End);

    // Word-swapped backends must start at an even guest offset and read an
    // even number of bytes. The scratch planner reserves the possible leading
    // and trailing padding around the requested MemoryReadable layout.
    function
        .instruction(&Instruction::LocalGet(word_swapped))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(-2))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalSet(aligned_address))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(-2))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::LocalSet(read_size))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::LocalGet(aligned_address))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(read_size))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_status_return(&mut function, 2);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(read_size))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(super::super::memarg()))
        .instruction(&Instruction::LocalSet(temporary_byte))
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(super::super::memarg()))
        .instruction(&Instruction::I32Store8(super::super::memarg()))
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(temporary_byte))
        .instruction(&Instruction::I32Store8(super::super::memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32And)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(super::super::memarg()))
        .instruction(&Instruction::I32Store8(super::super::memarg()))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    emit_status_return(&mut function, 1);
    function.instruction(&Instruction::End);

    // Fusion already exposes bytes in guest order.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(base))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(destination))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I64Const(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    function
}

fn emit_status_return(function: &mut Function, status: i64) {
    function
        .instruction(&Instruction::I64Const(status))
        .instruction(&Instruction::Return);
}
