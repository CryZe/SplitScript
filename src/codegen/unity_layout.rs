//! Validated IL2CPP target-memory layout facts.
//!
//! Wasm emitters consume this module instead of embedding Unity-version
//! offsets or object-layout facts in their instruction streams. High-level
//! discovery is source-defined in the standard library.

use std::collections::HashSet;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

#[derive(Debug, Clone, Copy)]
pub(super) struct UnityVersionLayout {
    /// `0` denotes ASR's unversioned/base IL2CPP layout.
    pub version: u32,
    pub class_field_count_offset: u64,
    pub class_static_table_offset: u64,
}

/// The first row is the explicit base layout and is also the fallback used by
/// generated offset selection after attachment has validated the version.
pub(super) const VERSION_LAYOUTS: [UnityVersionLayout; 4] = [
    UnityVersionLayout {
        version: 0,
        class_field_count_offset: 0x114,
        class_static_table_offset: 0xb8,
    },
    UnityVersionLayout {
        version: 2019,
        class_field_count_offset: 0x11c,
        class_static_table_offset: 0xb8,
    },
    UnityVersionLayout {
        version: 2020,
        class_field_count_offset: 0x120,
        class_static_table_offset: 0xb8,
    },
    UnityVersionLayout {
        version: 2022,
        class_field_count_offset: 0x124,
        class_static_table_offset: 0xb8,
    },
];

/// The implemented discovery algorithm and object layouts are the 64-bit
/// IL2CPP family. A future 32-bit family should be a distinct descriptor set.
pub(super) const POINTER_SIZE: u32 = 8;

#[derive(Debug, Clone, Copy)]
pub(super) struct Il2CppObjectLayout {
    pub assemblies_range_size: u32,
    pub assembly_image_offset: u64,
    pub assembly_name_offset: u64,
    pub image_type_count_offset: u64,
    pub image_type_count_size: u32,
    pub image_metadata_handle_offset: u64,
    pub metadata_handle_size: u32,
    pub class_name_offset: u64,
    pub class_namespace_offset: u64,
    pub class_fields_offset: u64,
    pub class_parent_offset: u64,
    pub class_field_count_size: u32,
    pub field_stride: u64,
    pub field_name_offset: u64,
    pub field_value_offset: u64,
    pub field_value_size: u32,
}

pub(super) const OBJECT_LAYOUT: Il2CppObjectLayout = Il2CppObjectLayout {
    assemblies_range_size: 16,
    assembly_image_offset: 0,
    assembly_name_offset: 0x18,
    image_type_count_offset: 0x18,
    image_type_count_size: 4,
    image_metadata_handle_offset: 0x28,
    metadata_handle_size: 4,
    class_name_offset: 0x10,
    class_namespace_offset: 0x18,
    class_fields_offset: 0x80,
    class_parent_offset: 0x58,
    class_field_count_size: 2,
    field_stride: 0x20,
    field_name_offset: 0,
    field_value_offset: 0x18,
    field_value_size: 4,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum VersionedOffset {
    ClassFieldCount,
    ClassStaticTable,
}

impl VersionedOffset {
    const fn get(self, layout: UnityVersionLayout) -> u64 {
        match self {
            Self::ClassFieldCount => layout.class_field_count_offset,
            Self::ClassStaticTable => layout.class_static_table_offset,
        }
    }
}

/// Selects a versioned offset as `i64`. The supplied local stores the Unity
/// version as `i64`, which lets async lowering reuse its address scratch local.
pub(super) fn emit_versioned_offset(
    function: &mut Function,
    version_i64_local: u32,
    offset: VersionedOffset,
) {
    for layout in VERSION_LAYOUTS[1..].iter().rev() {
        function
            .instruction(&Instruction::LocalGet(version_i64_local))
            .instruction(&Instruction::I64Const(i64_from_u64(layout.version.into())))
            .instruction(&Instruction::I64Eq)
            .instruction(&Instruction::If(BlockType::Result(ValType::I64)))
            .instruction(&Instruction::I64Const(i64_from_u64(offset.get(*layout))))
            .instruction(&Instruction::Else);
    }
    function.instruction(&Instruction::I64Const(i64_from_u64(
        offset.get(VERSION_LAYOUTS[0]),
    )));
    for _ in &VERSION_LAYOUTS[1..] {
        function.instruction(&Instruction::End);
    }
}

pub(super) fn validate() -> Vec<String> {
    let mut errors = Vec::new();
    let mut versions = HashSet::new();
    if VERSION_LAYOUTS.first().map(|layout| layout.version) != Some(0) {
        errors.push("the first Unity layout must be the explicit base version 0".to_owned());
    }
    for layout in VERSION_LAYOUTS {
        if !versions.insert(layout.version) {
            errors.push(format!("duplicate Unity layout version {}", layout.version));
        }
        for (name, offset, alignment) in [
            ("class field count", layout.class_field_count_offset, 2),
            (
                "class static table",
                layout.class_static_table_offset,
                u64::from(POINTER_SIZE),
            ),
        ] {
            if offset % alignment != 0 {
                errors.push(format!(
                    "Unity {} {name} offset {offset:#x} is not {alignment}-byte aligned",
                    layout.version
                ));
            }
        }
    }

    if !POINTER_SIZE.is_power_of_two() {
        errors.push(format!(
            "Unity pointer size {POINTER_SIZE} is not a power of two"
        ));
    }
    if OBJECT_LAYOUT.assemblies_range_size != POINTER_SIZE * 2 {
        errors.push("the assemblies range must contain exactly two pointers".to_owned());
    }
    for (name, offset) in [
        ("assembly image", OBJECT_LAYOUT.assembly_image_offset),
        ("assembly name", OBJECT_LAYOUT.assembly_name_offset),
        (
            "image metadata handle",
            OBJECT_LAYOUT.image_metadata_handle_offset,
        ),
        ("class name", OBJECT_LAYOUT.class_name_offset),
        ("class namespace", OBJECT_LAYOUT.class_namespace_offset),
        ("class fields", OBJECT_LAYOUT.class_fields_offset),
        ("class parent", OBJECT_LAYOUT.class_parent_offset),
        ("field name", OBJECT_LAYOUT.field_name_offset),
    ] {
        if offset % u64::from(POINTER_SIZE) != 0 {
            errors.push(format!(
                "Unity {name} offset {offset:#x} is not pointer aligned"
            ));
        }
    }
    for (name, size) in [
        ("image type count", OBJECT_LAYOUT.image_type_count_size),
        ("metadata handle", OBJECT_LAYOUT.metadata_handle_size),
        ("class field count", OBJECT_LAYOUT.class_field_count_size),
        ("field value", OBJECT_LAYOUT.field_value_size),
    ] {
        if !size.is_power_of_two() || size > POINTER_SIZE {
            errors.push(format!(
                "Unity {name} size {size} is not a supported scalar"
            ));
        }
    }
    if !OBJECT_LAYOUT
        .field_stride
        .is_multiple_of(u64::from(POINTER_SIZE))
        || OBJECT_LAYOUT
            .field_value_offset
            .checked_add(u64::from(OBJECT_LAYOUT.field_value_size))
            .is_none_or(|end| end > OBJECT_LAYOUT.field_stride)
    {
        errors.push("Unity field layout does not fit its declared stride".to_owned());
    }

    errors
}

const fn i64_from_u64(value: u64) -> i64 {
    assert!(
        value <= i64::MAX as u64,
        "target offset must fit an i64 immediate"
    );
    value as i64
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn unity_domain_descriptors_are_valid() {
        assert_eq!(validate(), Vec::<String>::new());
    }

    #[test]
    fn wasm_emitters_do_not_redeclare_unity_layout_facts() {
        let emitters = [
            include_str!("async_state.rs"),
            include_str!("data_plan.rs"),
            include_str!("runtime_helpers/unity.rs"),
        ];
        for forbidden in ["0x114", "0x11c", "0x120", "0x124", "0xb8"] {
            assert!(
                emitters.iter().all(|source| !source.contains(forbidden)),
                "Unity layout fact {forbidden:?} escaped its domain descriptor"
            );
        }
    }
}
