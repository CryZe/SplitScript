//! Compiler-generated operations for concrete growable source arrays.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{ast::ArrayTypeId, types::ResolvedArrayType};

use super::{GcLayout, Type, array_value};

#[derive(Debug, Default)]
pub(super) struct ArrayFunctions {
    pushes: HashMap<ArrayTypeId, u32>,
}

impl ArrayFunctions {
    pub(super) fn insert_push(&mut self, array: ArrayTypeId, function: u32) {
        self.pushes.insert(array, function);
    }

    pub(super) fn push(&self, array: ArrayTypeId) -> u32 {
        self.pushes[&array]
    }
}

pub(super) fn compile(
    arrays: &[ResolvedArrayType],
    plans: &ArrayFunctions,
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
) -> Vec<Function> {
    arrays
        .iter()
        .filter(|array| plans.pushes.contains_key(&array.id))
        .map(|array| compile_push(array, arrays, semantics, gc))
        .collect()
}

fn compile_push(
    array: &ResolvedArrayType,
    arrays: &[ResolvedArrayType],
    semantics: &crate::semantic::SemanticModel,
    gc: &GcLayout,
) -> Function {
    debug_assert!(array.length.is_none());
    let storage = array_value::storage_id(array.id, arrays, semantics);
    let storage_type = gc.val_type(Type::ArrayStorage(storage));

    // Parameters: array, value. Locals: backing, replacement, length, capacity.
    let mut function = Function::new([(2, storage_type), (2, ValType::I32)]);
    let backing = 2;
    let replacement = 3;
    let length = 4;
    let capacity = 5;
    let array_type = gc.index(Type::Array(array.id));
    let storage_type = gc.index(Type::ArrayStorage(storage));

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::StructGet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(capacity))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Eq)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Result(ValType::I32)))
        .instruction(&Instruction::I32Const(4))
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(capacity))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Mul)
        .instruction(&Instruction::End)
        .instruction(&Instruction::ArrayNewDefault(storage_type))
        .instruction(&Instruction::LocalTee(replacement))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::ArrayCopy {
            array_type_index_dst: storage_type,
            array_type_index_src: storage_type,
        })
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::BACKING_FIELD,
        })
        .instruction(&Instruction::LocalGet(replacement))
        .instruction(&Instruction::LocalSet(backing))
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(backing))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::ArraySet(storage_type))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::StructSet {
            struct_type_index: array_type,
            field_index: array_value::LENGTH_FIELD,
        })
        .instruction(&Instruction::End);
    function
}
