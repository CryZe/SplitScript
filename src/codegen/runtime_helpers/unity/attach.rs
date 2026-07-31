//! Unity/IL2CPP module attachment and metadata-root discovery.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use crate::{abi::AbiImportId, stdlib::StdlibTypeId};

use super::super::super::GcLayout;
use super::super::super::data_plan::{SignatureEntry, SignaturePool, StringPool};
use super::super::super::imports::Abi;
use super::super::super::unity_layout::{
    DISCOVERY_LAYOUT, DiscoverySignatureId, POINTER_SIZE, discovery_signature,
    emit_supported_version,
};
pub(in crate::codegen::runtime_helpers) fn compile_unity_attach(
    abi: &Abi,
    strings: &StringPool,
    signatures: &SignaturePool,
    scan_process_range: u32,
    read_relative32: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(11, ValType::I64)]);
    let process = 0;
    let version = 1;
    let module_address = 2;
    let module_size = 3;
    let assemblies_match = 4;
    let assemblies = 5;
    let metadata = 6;
    let end = 7;
    let cursor = 8;
    let lea_match = 9;
    let lea = 10;
    let shr = 11;
    let type_info = 12;
    let (module_name, module_name_len) = strings.get(DISCOVERY_LAYOUT.module_name);
    let assemblies_signature =
        signatures.get(discovery_signature(DiscoverySignatureId::Assemblies));
    let metadata_signature =
        signatures.get(discovery_signature(DiscoverySignatureId::MetadataFileName));
    let lea_signature =
        signatures.get(discovery_signature(DiscoverySignatureId::MetadataReference));
    let shr_signature = signatures.get(discovery_signature(DiscoverySignatureId::TypeInfoShift));
    let rax_signature = signatures.get(discovery_signature(DiscoverySignatureId::TypeInfoStore));

    // Only layouts represented by the validated IL2CPP version table are accepted.
    emit_supported_version(&mut function, version);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(module_name as i32))
        .instruction(&Instruction::I32Const(module_name_len as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleAddress),
        ))
        .instruction(&Instruction::LocalTee(module_address))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::I32Const(module_name as i32))
        .instruction(&Instruction::I32Const(module_name_len as i32))
        .instruction(&Instruction::Call(
            abi.function(AbiImportId::ProcessGetModuleSize),
        ))
        .instruction(&Instruction::LocalTee(module_size))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function.instruction(&Instruction::End);

    emit_static_scan_call(
        &mut function,
        process,
        module_address,
        module_size,
        assemblies_signature,
        scan_process_range,
    );
    function
        .instruction(&Instruction::LocalTee(assemblies_match))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assemblies_match))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.assemblies_displacement_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalTee(assemblies))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function.instruction(&Instruction::End);

    emit_static_scan_call(
        &mut function,
        process,
        module_address,
        module_size,
        metadata_signature,
        scan_process_range,
    );
    function
        .instruction(&Instruction::LocalTee(metadata))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(module_address))
        .instruction(&Instruction::LocalGet(module_size))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(end))
        .instruction(&Instruction::LocalGet(module_address))
        .instruction(&Instruction::LocalSet(cursor))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::LocalGet(end))
        .instruction(&Instruction::LocalGet(cursor))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I32Const(lea_signature.needle as i32))
        .instruction(&Instruction::I32Const(lea_signature.mask as i32))
        .instruction(&Instruction::I32Const(lea_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::LocalTee(lea_match))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.metadata_reference_displacement_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalGet(metadata))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.metadata_reference_displacement_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(lea))
        .instruction(&Instruction::Br(2))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(lea_match))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(cursor))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(lea))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.type_info_shift_scan_size as i64,
        ))
        .instruction(&Instruction::I32Const(shr_signature.needle as i32))
        .instruction(&Instruction::I32Const(shr_signature.mask as i32))
        .instruction(&Instruction::I32Const(shr_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::LocalTee(shr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(shr))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.instruction_displacement_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(shr))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(shr))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.type_info_store_scan_size as i64,
        ))
        .instruction(&Instruction::I32Const(rax_signature.needle as i32))
        .instruction(&Instruction::I32Const(rax_signature.mask as i32))
        .instruction(&Instruction::I32Const(rax_signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range))
        .instruction(&Instruction::I64Const(
            DISCOVERY_LAYOUT.instruction_displacement_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(type_info))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(type_info))
        .instruction(&Instruction::Call(read_relative32))
        .instruction(&Instruction::LocalTee(type_info))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_attach_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(assemblies))
        .instruction(&Instruction::LocalGet(type_info))
        .instruction(&Instruction::LocalGet(version))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityModule),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_static_scan_call(
    function: &mut Function,
    process: u32,
    address: u32,
    size: u32,
    signature: SignatureEntry,
    scan_process_range: u32,
) {
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::LocalGet(size))
        .instruction(&Instruction::I32Const(signature.needle as i32))
        .instruction(&Instruction::I32Const(signature.mask as i32))
        .instruction(&Instruction::I32Const(signature.len as i32))
        .instruction(&Instruction::Call(scan_process_range));
}

fn emit_unity_attach_failure(function: &mut Function, gc: &GcLayout) {
    function
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityModule),
        )))
        .instruction(&Instruction::Return);
}
