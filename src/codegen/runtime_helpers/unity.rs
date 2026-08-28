//! Unity/IL2CPP type and field discovery runtime helpers.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, RefType, ValType};

use crate::{
    abi::AbiImportId,
    stdlib::{StdlibFieldId, StdlibTypeId},
};

use super::super::imports::Abi;
use super::super::memory_plan::{AbiReadScratch, ScratchRegion};
use super::super::unity_layout::{
    OBJECT_LAYOUT, POINTER_SIZE, VersionedOffset, emit_versioned_offset,
};

use super::super::{GcLayout, Type, emit_array_get, memarg};

const LOOKUP_RETRY: i32 = 0;
const LOOKUP_MISSING: i32 = 1;
const LOOKUP_FOUND: i32 = 2;
const LOOKUP_AMBIGUOUS: i32 = 3;

/// Stops the surrounding class traversal when the remote C string exactly
/// matches an internal IL2CPP boundary name. A failed metadata read also ends
/// traversal, matching ASR's best-effort parent iterator.
fn emit_break_if_c_string_equals_literal(
    function: &mut Function,
    abi: &Abi,
    abi_read: AbiReadScratch,
    process: u32,
    address: u32,
    expected: &[u8],
    break_depth: u32,
) {
    let length = expected.len() + 1;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Const(length as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(break_depth))
        .instruction(&Instruction::I32Const(1));
    for (index, byte) in expected.iter().copied().chain([0]).enumerate() {
        function
            .instruction(&Instruction::I32Const(abi_read.start() + index as i32))
            .instruction(&Instruction::I32Load8U(memarg()))
            .instruction(&Instruction::I32Const(byte as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::BrIf(break_depth));
}

pub(super) fn compile_c_string_eq(abi: &Abi, gc: &GcLayout, c_string: ScratchRegion) -> Function {
    let c_string_start = c_string.start();
    let mut function = Function::new([(1, ValType::I32)]);
    let process = 0;
    let address = 1;
    let expected = 2;
    let start = 3;
    let len = 4;
    let index = 5;
    function
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(255))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(c_string_start))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(c_string_start))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(c_string_start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(start))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

/// Matches the C# compiler pattern `<Name>k__BackingField` without exposing
/// that metadata spelling to SplitScript programs.
pub(super) fn compile_backing_field_eq(
    abi: &Abi,
    gc: &GcLayout,
    c_string: ScratchRegion,
) -> Function {
    let c_string_start = c_string.start();
    let mut function = Function::new([(2, ValType::I32)]);
    let process = 0;
    let address = 1;
    let expected = 2;
    let expected_len = 3;
    let index = 4;
    function
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(expected_len))
        .instruction(&Instruction::I32Const(237))
        .instruction(&Instruction::I32GtU)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(address))
        .instruction(&Instruction::I32Const(c_string_start))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Const(18))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(c_string_start))
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::I32Const(b'<' as i32))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(c_string.at(2)))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::I64Const(i64::from_le_bytes(*b"k__Backi")))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(c_string.at(10)))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::I64Const(i64::from_le_bytes(*b"ngField\0")))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(expected_len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(c_string.at(1)))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::I32Load8U(memarg()))
        .instruction(&Instruction::LocalGet(expected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_unity_get_image(
    abi: &Abi,
    c_string_eq: u32,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(6, ValType::I64)]);
    let process = 0;
    let module = 1;
    let expected_name = 2;
    let first = 3;
    let limit = 4;
    let index = 5;
    let assembly = 6;
    let name_ptr = 7;
    let image = 8;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityModule),
            field_index: gc.standard_field_index(StdlibFieldId::UnityModuleAssemblies),
        })
        .instruction(&Instruction::I32Const(
            abi_read.destination(OBJECT_LAYOUT.assemblies_range_size),
        ))
        .instruction(&Instruction::I32Const(
            OBJECT_LAYOUT.assemblies_range_size as i32,
        ))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityImage),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(first))
        .instruction(&Instruction::I32Const(abi_read.at(POINTER_SIZE)))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(limit))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(POINTER_SIZE as i64))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalGet(limit))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(first))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(POINTER_SIZE as i64))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(assembly))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assembly))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.assembly_name_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(assembly))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.assembly_image_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(image))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(image))
        .instruction(&Instruction::LocalGet(module))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityImage),
        ))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityImage),
        )))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_unity_get_class(
    abi: &Abi,
    c_string_eq: u32,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(9, ValType::I64), (3, ValType::I32)]);
    let process = 0;
    let image_value = 1;
    let expected_name = 2;
    let metadata_ptr = 3;
    let type_info_table = 4;
    let classes = 5;
    let count = 6;
    let index = 7;
    let class = 8;
    let name_ptr = 9;
    let namespace_ptr = 10;
    let selected = 11;
    let dot_plus_one = 12;
    let scan_index = 13;
    let metadata_handle = 14;

    // Find the last namespace separator in the requested class name.
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Const(b'.' as i32))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(dot_plus_one))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(scan_index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(scan_index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);

    // Read the image's type count.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageAddress),
        })
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.image_type_count_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(
            abi_read.destination(OBJECT_LAYOUT.image_type_count_size),
        ))
        .instruction(&Instruction::I32Const(
            OBJECT_LAYOUT.image_type_count_size as i32,
        ))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalTee(count))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function.instruction(&Instruction::End);

    // V2020+ images point at a metadata handle, whose u32 selects the first
    // class pointer from the global type-info-definition table.
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageAddress),
        })
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.image_metadata_handle_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(metadata_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(metadata_ptr))
        .instruction(&Instruction::I32Const(
            abi_read.destination(OBJECT_LAYOUT.metadata_handle_size),
        ))
        .instruction(&Instruction::I32Const(
            OBJECT_LAYOUT.metadata_handle_size as i32,
        ))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::LocalSet(metadata_handle))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageModule),
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityModule),
            field_index: gc.standard_field_index(StdlibFieldId::UnityModuleTypeInfoTable),
        })
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(type_info_table))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty));
    emit_unity_class_failure(&mut function, gc);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(type_info_table))
        .instruction(&Instruction::LocalGet(metadata_handle))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Const(POINTER_SIZE as i64))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(classes))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(classes))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(POINTER_SIZE as i64))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(class))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_name_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_namespace_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(namespace_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(namespace_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(dot_plus_one))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::LocalSet(selected))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_AMBIGUOUS))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityClass),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_MISSING))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityClass),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::LocalGet(image_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageModule),
        })
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityClass),
        ))
        .instruction(&Instruction::End);
    function
}

fn emit_unity_class_failure(function: &mut Function, gc: &GcLayout) {
    function
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityClass),
        )))
        .instruction(&Instruction::Return);
}

/// Returns a lookup status followed by `field_offset + 1`. A completed miss is
/// distinct from a transient process-memory failure, while the encoded offset
/// keeps a real field at offset zero representable.
pub(super) fn compile_unity_get_field_offset(
    abi: &Abi,
    c_string_eq: u32,
    backing_field_eq: u32,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([(10, ValType::I64), (1, ValType::I32)]);
    let process = 0;
    let class_value = 1;
    let expected_name = 2;
    let current = 3;
    let fields = 4;
    let count = 5;
    let index = 6;
    let field = 7;
    let name_ptr = 8;
    let parent = 9;
    let encoded = 10;
    let field_count_offset = 11;
    let selected_encoded = 12;
    let comparison = 13;
    function
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassAddress),
        })
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassModule),
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityModule),
            field_index: gc.standard_field_index(StdlibFieldId::UnityModuleVersion),
        })
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(field_count_offset));
    emit_versioned_offset(
        &mut function,
        field_count_offset,
        VersionedOffset::ClassFieldCount,
    );
    function
        .instruction(&Instruction::LocalSet(field_count_offset))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1))
        // ASR only walks user-defined inheritance. UnityEngine base classes
        // use metadata shapes outside the game image and must not be scanned
        // as though they were another managed game class.
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_name_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1));
    emit_break_if_c_string_equals_literal(
        &mut function,
        abi,
        abi_read,
        process,
        name_ptr,
        b"Object",
        1,
    );
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_namespace_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(1));
    emit_break_if_c_string_equals_literal(
        &mut function,
        abi,
        abi_read,
        process,
        name_ptr,
        b"UnityEngine",
        1,
    );
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::LocalGet(field_count_offset))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(
            abi_read.destination(OBJECT_LAYOUT.class_field_count_size),
        ))
        .instruction(&Instruction::I32Const(
            OBJECT_LAYOUT.class_field_count_size as i32,
        ))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load16U(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(count))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64Const(u16::MAX as i64))
        .instruction(&Instruction::I64Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(count))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_fields_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(fields))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(count))
        .instruction(&Instruction::I64GeU)
        .instruction(&Instruction::BrIf(1))
        // Match ASR's field iterator: an individual unreadable metadata entry
        // is not evidence that the class itself is unavailable. Skip that
        // entry and keep traversing the remaining declared slots. Structural
        // reads and the offset of an entry whose name matched still retry.
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::LocalGet(fields))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(OBJECT_LAYOUT.field_stride as i64))
        .instruction(&Instruction::I64Mul)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(field))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.field_name_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(name_ptr))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::Call(c_string_eq))
        .instruction(&Instruction::LocalTee(comparison))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::BrIf(0))
        .instruction(&Instruction::LocalGet(comparison))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(name_ptr))
        .instruction(&Instruction::LocalGet(expected_name))
        .instruction(&Instruction::Call(backing_field_eq))
        .instruction(&Instruction::LocalTee(comparison))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(comparison))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.field_value_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(
            abi_read.destination(OBJECT_LAYOUT.field_value_size),
        ))
        .instruction(&Instruction::I32Const(
            OBJECT_LAYOUT.field_value_size as i32,
        ))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I32Load(memarg()))
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(encoded))
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::LocalSet(selected_encoded))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_AMBIGUOUS))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(current))
        .instruction(&Instruction::I64Const(
            OBJECT_LAYOUT.class_parent_offset as i64,
        ))
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalSet(parent))
        .instruction(&Instruction::LocalGet(parent))
        .instruction(&Instruction::LocalSet(current))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(LOOKUP_MISSING))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_unity_get_field_any(
    unity_get_field_offset: u32,
    names_array: u32,
    names_storage: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([
        (3, ValType::I32),
        (2, ValType::I64),
        (
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(names_storage),
            }),
        ),
    ]);
    let process = 0;
    let class_value = 1;
    let names = 2;
    let index = 3;
    let status = 4;
    let selected_index = 5;
    let encoded = 6;
    let selected_encoded = 7;
    let names_backing = 8;
    function
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: names_array,
            field_index: super::super::array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(names_backing))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: names_array,
            field_index: super::super::array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::LocalGet(names_backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        names_storage,
        Type::Standard(StdlibTypeId::String),
        gc,
    );
    function
        .instruction(&Instruction::Call(unity_get_field_offset))
        .instruction(&Instruction::LocalSet(encoded))
        .instruction(&Instruction::LocalSet(status))
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityField),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_AMBIGUOUS))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityField),
        ))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::LocalSet(selected_encoded))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalSet(selected_index))
        .instruction(&Instruction::Else)
        // Two aliases resolving to the same metadata field are one runtime
        // match, not an ambiguity (for example a property name and its
        // explicitly listed backing-field spelling).
        .instruction(&Instruction::LocalGet(encoded))
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::I32Const(-1))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityField),
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
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_MISSING))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityField),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::LocalGet(selected_encoded))
        .instruction(&Instruction::I64Const(1))
        .instruction(&Instruction::I64Sub)
        .instruction(&Instruction::I32WrapI64)
        .instruction(&Instruction::LocalGet(selected_index))
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityField),
        ))
        .instruction(&Instruction::End);
    function
}

/// Probes every schema alias and returns a completed missing, unique, or
/// ambiguous result. A class whose address is zero is the private ambiguity
/// sentinel consumed by the source-defined binding policy. Transient memory
/// failures remain distinct and suspend the caller.
pub(super) fn compile_unity_get_class_any(
    unity_get_class: u32,
    names_array: u32,
    names_storage: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([
        (2, ValType::I32),
        (
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(names_storage),
            }),
        ),
        (
            2,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(gc.standard_index(StdlibTypeId::UnityClass)),
            }),
        ),
    ]);
    let process = 0;
    let image = 1;
    let names = 2;
    let index = 3;
    let status = 4;
    let names_backing = 5;
    let class = 6;
    let selected = 7;
    function
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: names_array,
            field_index: super::super::array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(names_backing))
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: names_array,
            field_index: super::super::array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(image))
        .instruction(&Instruction::LocalGet(names_backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        names_storage,
        Type::Standard(StdlibTypeId::String),
        gc,
    );
    function
        .instruction(&Instruction::Call(unity_get_class))
        .instruction(&Instruction::LocalSet(class))
        .instruction(&Instruction::LocalSet(status))
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_RETRY))
        .instruction(&Instruction::RefNull(HeapType::Concrete(
            gc.standard_index(StdlibTypeId::UnityClass),
        )))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_AMBIGUOUS))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(image))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageModule),
        })
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityClass),
        ))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::LocalSet(selected))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassAddress),
        })
        .instruction(&Instruction::LocalGet(class))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassAddress),
        })
        .instruction(&Instruction::I64Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::LocalGet(image))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityImage),
            field_index: gc.standard_field_index(StdlibFieldId::UnityImageModule),
        })
        .instruction(&Instruction::StructNew(
            gc.standard_index(StdlibTypeId::UnityClass),
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
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(LOOKUP_MISSING))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(selected))
        .instruction(&Instruction::End);
    function
}

pub(super) fn compile_unity_get_static_instance(
    abi: &Abi,
    unity_get_field_any: u32,
    gc: &GcLayout,
    abi_read: AbiReadScratch,
) -> Function {
    let mut function = Function::new([
        (1, gc.val_type(Type::Standard(StdlibTypeId::UnityField))),
        (1, ValType::I64),
        (1, ValType::I32),
    ]);
    let process = 0;
    let class_value = 1;
    let names = 2;
    let field = 3;
    let static_table = 4;
    let status = 5;
    function
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::LocalGet(names))
        .instruction(&Instruction::Call(unity_get_field_any))
        .instruction(&Instruction::LocalSet(field))
        .instruction(&Instruction::LocalSet(status))
        .instruction(&Instruction::LocalGet(status))
        .instruction(&Instruction::I32Const(LOOKUP_FOUND))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassModule),
        })
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityModule),
            field_index: gc.standard_field_index(StdlibFieldId::UnityModuleVersion),
        })
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::LocalSet(static_table))
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(class_value))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityClass),
            field_index: gc.standard_field_index(StdlibFieldId::UnityClassAddress),
        });
    emit_versioned_offset(
        &mut function,
        static_table,
        VersionedOffset::ClassStaticTable,
    );
    function
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::LocalTee(static_table))
        .instruction(&Instruction::I64Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(process))
        .instruction(&Instruction::LocalGet(static_table))
        .instruction(&Instruction::LocalGet(field))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: gc.standard_index(StdlibTypeId::UnityField),
            field_index: gc.standard_field_index(StdlibFieldId::UnityFieldOffset),
        })
        .instruction(&Instruction::I64ExtendI32U)
        .instruction(&Instruction::I64Add)
        .instruction(&Instruction::I32Const(abi_read.destination(POINTER_SIZE)))
        .instruction(&Instruction::I32Const(POINTER_SIZE as i32))
        .instruction(&Instruction::Call(abi.function(AbiImportId::ProcessRead)))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I64Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(abi_read.start()))
        .instruction(&Instruction::I64Load(memarg()))
        .instruction(&Instruction::End);
    function
}
