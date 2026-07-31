//! Game Boy Advance emulator discovery and hardware-address translation.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::GcLayout;
use super::super::data_plan::{SignatureEntry, SignaturePool, StringPool};
use super::super::gba_layout;
use super::super::imports::Abi;
use super::super::memory_plan::AbiReadScratch;

const BACKEND_STABLE: i32 = 1;
const BACKEND_POINTER_32: i32 = 2;
const BACKEND_POINTER_64: i32 = 3;
const BACKEND_NOCASH: i32 = 4;

pub(super) fn compile_attach(
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
    scan_process_range: u32,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(3, ValType::I32), (10, ValType::I64)]);
    let process = 0;
    let index = 1;
    let byte = 2;
    let displacement = 3;
    let count = 4;
    let address = 5;
    let flags = 6;
    let module = 7;
    let module_size = 8;
    let matched = 9;
    let pointer = 10;
    let ewram = 11;
    let iwram = 12;
    let temporary = 13;

    // VisualBoyAdvance and VBA-M.
    for name in ["visualboyadvance-m.exe", "VisualBoyAdvance.exe"] {
        emit_module_address(&mut function, abi, strings, process, name, module);
        function
            .instruction(&Instruction::LocalGet(module))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_module_size(&mut function, abi, strings, process, name, module_size, gc);
        emit_vba_mapping(
            &mut function,
            abi,
            signatures,
            scan_process_range,
            abi_read,
            module,
            module_size,
            matched,
            pointer,
            ewram,
            iwram,
            index,
            byte,
            displacement,
            true,
            gc,
            gba_layout::VBA_X64_EWRAM,
            gba_layout::VBA_X64_IWRAM,
        );
        emit_pointer_mapping_return(&mut function, index, ewram, iwram, gc);
        function.instruction(&Instruction::End);
    }

    // NO$GBA.
    emit_module_address(&mut function, abi, strings, process, "NO$GBA.EXE", module);
    function
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_module_size(
        &mut function,
        abi,
        strings,
        process,
        "NO$GBA.EXE",
        module_size,
        gc,
    );
    emit_required_scan(
        &mut function,
        signatures.get(gba_layout::NOCASH_BASE),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    emit_read_u32(&mut function, abi, abi_read, matched, 2, pointer, gc);
    emit_nocash_mapping_return(&mut function, pointer, gc);
    function.instruction(&Instruction::End);

    // Standalone Mednafen uses the same layouts as the VBA-derived cores.
    emit_module_address(&mut function, abi, strings, process, "mednafen.exe", module);
    function
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_module_size(
        &mut function,
        abi,
        strings,
        process,
        "mednafen.exe",
        module_size,
        gc,
    );
    emit_vba_mapping(
        &mut function,
        abi,
        signatures,
        scan_process_range,
        abi_read,
        module,
        module_size,
        matched,
        pointer,
        ewram,
        iwram,
        index,
        byte,
        displacement,
        false,
        gc,
        gba_layout::SHARED_X64_EWRAM,
        gba_layout::SHARED_X64_IWRAM,
    );
    emit_pointer_mapping_return(&mut function, index, ewram, iwram, gc);
    function.instruction(&Instruction::End);

    // RetroArch VBA-derived cores can be discovered directly from the core
    // module. The mGBA core falls through to contiguous-range discovery.
    for name in [
        "vbam_libretro.dll",
        "mednafen_gba_libretro.dll",
        "vba_next_libretro.dll",
    ] {
        emit_module_address(&mut function, abi, strings, process, name, module);
        function
            .instruction(&Instruction::LocalGet(module))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_module_size(&mut function, abi, strings, process, name, module_size, gc);
        emit_vba_mapping(
            &mut function,
            abi,
            signatures,
            scan_process_range,
            abi_read,
            module,
            module_size,
            matched,
            pointer,
            ewram,
            iwram,
            index,
            byte,
            displacement,
            false,
            gc,
            gba_layout::SHARED_X64_EWRAM,
            gba_layout::SHARED_X64_IWRAM,
        );
        emit_pointer_mapping_return(&mut function, index, ewram, iwram, gc);
        function.instruction(&Instruction::End);
    }

    // gpSP keeps both RAM regions relative to a separately discovered base.
    emit_module_address(
        &mut function,
        abi,
        strings,
        process,
        "gpsp_libretro.dll",
        module,
    );
    function
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_module_size(
        &mut function,
        abi,
        strings,
        process,
        "gpsp_libretro.dll",
        module_size,
        gc,
    );
    emit_scan(
        &mut function,
        signatures.get(gba_layout::GPSP_BASE_X64),
        scan_process_range,
        module,
        module_size,
        matched,
    );
    function
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_required_scan(
        &mut function,
        signatures.get(gba_layout::GPSP_BASE_X86),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    emit_read_u32(&mut function, abi, abi_read, matched, 1, temporary, gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(pointer));
    emit_read_i32(&mut function, abi, abi_read, pointer, 0, displacement, gc);
    function
        .instruction(&Instruction::LocalGet(pointer))
        .instruction(&Instruction::I64Const(4))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(displacement))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(pointer));
    emit_read_u64(&mut function, abi, abi_read, pointer, 0, temporary, gc);
    function.instruction(&Instruction::End);
    emit_required_scan(
        &mut function,
        signatures.get(gba_layout::GPSP_EWRAM),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    emit_read_i32(&mut function, abi, abi_read, matched, 8, displacement, gc);
    function
        .instruction(&Instruction::LocalGet(temporary))
        .instruction(&Instruction::LocalGet(displacement))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(ewram));
    emit_required_scan(
        &mut function,
        signatures.get(gba_layout::GPSP_IWRAM),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    emit_read_i32(&mut function, abi, abi_read, matched, 9, displacement, gc);
    function
        .instruction(&Instruction::LocalGet(temporary))
        .instruction(&Instruction::LocalGet(displacement))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(iwram));
    emit_mapping_return(&mut function, ewram, iwram, gc);
    function.instruction(&Instruction::End);

    // mGBA, BizHawk's mGBA core, and RetroArch's mGBA core expose one
    // contiguous 0x48000-byte readable/writable mapping.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeCount),
        ))
        .instruction(&Instruction::LocalSet(count))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeSize),
        ))
        .instruction(&Instruction::I64Const(0x48000))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeFlags),
        ))
        .instruction(&Instruction::LocalTee(flags))
        .instruction(&Instruction::I64Const(0x6))
        .instruction(&Instruction::I64And)
        .instruction(&Instruction::I64Const(0x6))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetMemoryRangeAddress),
        ))
        .instruction(&Instruction::LocalTee(address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(BACKEND_STABLE))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(0x40000))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::GbaEmulator),
        ))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
    emit_attach_failure(&mut function, gc);
    function.instruction(&Instruction::End);
    function
}

#[allow(clippy::too_many_arguments)]
fn emit_vba_mapping(
    function: &mut Function,
    abi: &Abi,
    signatures: &SignaturePool,
    scan_process_range: u32,
    abi_read: AbiReadScratch,
    module: u32,
    module_size: u32,
    matched: u32,
    pointer: u32,
    ewram: u32,
    iwram: u32,
    backend: u32,
    byte: u32,
    displacement: u32,
    allow_old_vba: bool,
    gc: &GcLayout,
    x64_ewram: &str,
    x64_iwram: &str,
) {
    emit_scan(
        function,
        signatures.get(x64_ewram),
        scan_process_range,
        module,
        module_size,
        matched,
    );
    function
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_scan(
        function,
        signatures.get(gba_layout::SHARED_X86_EWRAM),
        scan_process_range,
        module,
        module_size,
        matched,
    );
    if allow_old_vba {
        function
            .instruction(&Instruction::LocalGet(matched))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_required_scan(
            function,
            signatures.get(gba_layout::VBA_X86_OLD_EWRAM),
            scan_process_range,
            module,
            module_size,
            matched,
            gc,
        );
        emit_read_u32(function, abi, abi_read, matched, 8, ewram, gc);
        function
            .instruction(&Instruction::LocalGet(ewram))
            .instruction(&Instruction::I64Const(4))
            .instruction(&Instruction::I64Add)
            .instruction(&Instruction::LocalSet(iwram))
            .instruction(&Instruction::I32Const(BACKEND_POINTER_32))
            .instruction(&Instruction::LocalSet(backend));
        function.instruction(&Instruction::Br(1));
        function.instruction(&Instruction::End);
    } else {
        function
            .instruction(&Instruction::LocalGet(matched))
            .instruction(&Instruction::I64Eqz)
            .instruction(&Instruction::If(BlockType::Empty));
        emit_attach_failure(function, gc);
        function.instruction(&Instruction::End);
    }
    emit_read_u32(function, abi, abi_read, matched, 1, ewram, gc);
    emit_required_scan(
        function,
        signatures.get(gba_layout::SHARED_X86_IWRAM),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    emit_read_u32(function, abi, abi_read, matched, 1, iwram, gc);
    function
        .instruction(&Instruction::I32Const(BACKEND_POINTER_32))
        .instruction(&Instruction::LocalSet(backend));
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(pointer));
    emit_resolve_x64_pointer(
        function,
        abi,
        abi_read,
        pointer,
        ewram,
        byte,
        displacement,
        gc,
    );
    emit_required_scan(
        function,
        signatures.get(x64_iwram),
        scan_process_range,
        module,
        module_size,
        matched,
        gc,
    );
    function
        .instruction(&Instruction::LocalGet(matched))
        .instruction(&Instruction::I64Const(3))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(pointer));
    emit_resolve_x64_pointer(
        function,
        abi,
        abi_read,
        pointer,
        iwram,
        byte,
        displacement,
        gc,
    );
    function
        .instruction(&Instruction::I32Const(BACKEND_POINTER_64))
        .instruction(&Instruction::LocalSet(backend));
    function.instruction(&Instruction::End);
}

#[allow(clippy::too_many_arguments)]
fn emit_resolve_x64_pointer(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    pointer: u32,
    output: u32,
    byte: u32,
    displacement: u32,
    gc: &GcLayout,
) {
    // `pointer` still identifies the RIP-relative displacement here. The
    // opcode byte that selects one or two levels of indirection is relative
    // to that instruction, not to the resolved global address.
    emit_read_u8(function, abi, abi_read, pointer, 10, byte, gc);
    emit_read_i32(function, abi, abi_read, pointer, 0, displacement, gc);
    function
        .instruction(&Instruction::LocalGet(pointer))
        .instruction(&Instruction::I64Const(4))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(displacement))
        .instruction(&Instruction::I64ExtendI32S)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(output));
    function
        .instruction(&Instruction::LocalGet(byte))
        .instruction(&Instruction::I32Const(0x48))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_read_u64(function, abi, abi_read, output, 0, output, gc);
    function.instruction(&Instruction::End);
}

fn emit_module_address(
    function: &mut Function,
    abi: &Abi,
    strings: &StringPool,
    process: u32,
    name: &str,
    output: u32,
) {
    let (pointer, length) = strings.get(name);
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(pointer as i32))
        .instruction(&Instruction::I32Const(length as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleAddress),
        ))
        .instruction(&Instruction::LocalSet(output));
}

#[allow(clippy::too_many_arguments)]
fn emit_module_size(
    function: &mut Function,
    abi: &Abi,
    strings: &StringPool,
    process: u32,
    name: &str,
    output: u32,
    gc: &GcLayout,
) {
    let (pointer, length) = strings.get(name);
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(pointer as i32))
        .instruction(&Instruction::I32Const(length as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleSize),
        ))
        .instruction(&Instruction::LocalTee(output))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_attach_failure(function, gc);
    function.instruction(&Instruction::End);
}

fn emit_scan(
    function: &mut Function,
    signature: SignatureEntry,
    scan_process_range: u32,
    module: u32,
    module_size: u32,
    output: u32,
) {
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::LocalGet(module_size))
        .instruction(&Instruction::I32Const(signature.needle as i32))
        .instruction(&Instruction::I32Const(signature.mask as i32))
        .instruction(&Instruction::I32Const(signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::LocalSet(output));
}

fn emit_required_scan(
    function: &mut Function,
    signature: SignatureEntry,
    scan_process_range: u32,
    module: u32,
    module_size: u32,
    output: u32,
    gc: &GcLayout,
) {
    emit_scan(
        function,
        signature,
        scan_process_range,
        module,
        module_size,
        output,
    );
    function
        .instruction(&Instruction::LocalGet(output))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_attach_failure(function, gc);
    function.instruction(&Instruction::End);
}

fn emit_read_u8(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    offset: i64,
    output: u32,
    gc: &GcLayout,
) {
    emit_process_read(function, abi, abi_read, address, offset, 1, gc);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load8U(super::super::memarg()))
        .instruction(&Instruction::LocalSet(output));
}

fn emit_read_i32(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    offset: i64,
    output: u32,
    gc: &GcLayout,
) {
    emit_process_read(function, abi, abi_read, address, offset, 4, gc);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::LocalSet(output));
}

fn emit_read_u32(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    offset: i64,
    output: u32,
    gc: &GcLayout,
) {
    emit_process_read(function, abi, abi_read, address, offset, 4, gc);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(super::super::memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(output));
}

fn emit_read_u64(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    offset: i64,
    output: u32,
    gc: &GcLayout,
) {
    emit_process_read(function, abi, abi_read, address, offset, 8, gc);
    function
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(super::super::memarg()))
        .instruction(&Instruction::LocalSet(output));
}

fn emit_process_read(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    address: u32,
    offset: i64,
    size: u32,
    gc: &GcLayout,
) {
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I64Const(offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(size)))
        .instruction(&Instruction::I32Const(size as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_attach_failure(function, gc);
    function.instruction(&Instruction::End);
}

fn emit_mapping_return(function: &mut Function, ewram: u32, iwram: u32, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(BACKEND_STABLE))
        .instruction(&Instruction::LocalGet(ewram))
        .instruction(&Instruction::LocalGet(iwram))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::GbaEmulator),
        ))
        .instruction(&Instruction::Return);
}

fn emit_pointer_mapping_return(
    function: &mut Function,
    backend: u32,
    ewram_pointer: u32,
    iwram_pointer: u32,
    gc: &GcLayout,
) {
    function
        .instruction(&Instruction::LocalGet(backend))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(ewram_pointer))
        .instruction(&Instruction::LocalGet(iwram_pointer))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::GbaEmulator),
        ))
        .instruction(&Instruction::Return);
}

fn emit_nocash_mapping_return(function: &mut Function, base_pointer: u32, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(BACKEND_NOCASH))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(base_pointer))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::GbaEmulator),
        ))
        .instruction(&Instruction::Return);
}

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
    let emulator_type = gc.standard_index(StdlibTypeId::GbaEmulator);

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
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorBackend),
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
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorEwram),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorAux1),
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
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorIwram),
        })
        .instruction(&Instruction::LocalSet(base))
        .instruction(&Instruction::LocalGet(emulator))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: emulator_type,
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorAux2),
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
            field_index: gc.standard_field_index(crate::stdlib::StdlibFieldId::GbaEmulatorAux1),
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

fn emit_attach_failure(function: &mut Function, gc: &GcLayout) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::GbaEmulator),
        )))
        .instruction(&Instruction::Return);
}
