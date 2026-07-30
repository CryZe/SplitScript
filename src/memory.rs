//! Type-directed process-memory layouts.
//!
//! This module is the single source of truth for the `MemoryReadable`
//! capability. Type checking, documentation/editor queries, and WebAssembly
//! lowering all consume the same deterministic layouts.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{RecordDecl, RecordFieldId, RecordId},
    semantic::SemanticModel,
    types::{BuiltinType, TypeId, TypeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryFieldLayout {
    pub field: RecordFieldId,
    pub ty: TypeId,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordMemoryLayout {
    pub record: RecordId,
    pub size: u32,
    pub alignment: u32,
    pub fields: Vec<MemoryFieldLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTypeLayout<'a> {
    Scalar { size: u32, alignment: u32 },
    Record(&'a RecordMemoryLayout),
}

impl MemoryTypeLayout<'_> {
    pub fn size(self) -> u32 {
        match self {
            Self::Scalar { size, .. } => size,
            Self::Record(layout) => layout.size,
        }
    }

    pub fn alignment(self) -> u32 {
        match self {
            Self::Scalar { alignment, .. } => alignment,
            Self::Record(layout) => layout.alignment,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryLayouts {
    records: HashMap<RecordId, Result<RecordMemoryLayout, String>>,
}

impl MemoryLayouts {
    pub fn build(records: &[RecordDecl], semantics: &SemanticModel) -> Self {
        let mut layouts = Self::default();
        for record in records {
            let mut visiting = HashSet::new();
            let result = layouts.build_record(record.id, records, semantics, &mut visiting);
            layouts.records.entry(record.id).or_insert(result);
        }
        layouts
    }

    pub fn layout<'a>(
        &'a self,
        ty: TypeId,
        semantics: &SemanticModel,
    ) -> Result<MemoryTypeLayout<'a>, String> {
        match semantics.types().kind(ty) {
            TypeKind::Builtin(builtin) => scalar_layout(*builtin)
                .map(|(size, alignment)| MemoryTypeLayout::Scalar { size, alignment })
                .ok_or_else(|| format!("type `{builtin}` is not MemoryReadable")),
            TypeKind::Record(record) => self
                .records
                .get(record)
                .expect("every declared record has a memory-layout result")
                .as_ref()
                .map(MemoryTypeLayout::Record)
                .map_err(Clone::clone),
            kind => Err(format!("type `{kind:?}` is not MemoryReadable")),
        }
    }

    pub fn record(&self, record: RecordId) -> Result<&RecordMemoryLayout, &str> {
        self.records
            .get(&record)
            .expect("every declared record has a memory-layout result")
            .as_ref()
            .map_err(String::as_str)
    }

    fn build_record(
        &mut self,
        record: RecordId,
        records: &[RecordDecl],
        semantics: &SemanticModel,
        visiting: &mut HashSet<RecordId>,
    ) -> Result<RecordMemoryLayout, String> {
        if let Some(layout) = self.records.get(&record) {
            return layout.clone();
        }
        if !visiting.insert(record) {
            return Err("recursive records do not have a finite process-memory layout".to_owned());
        }

        let declaration = records
            .iter()
            .find(|declaration| declaration.id == record)
            .expect("record IDs refer to declarations");
        if declaration.fields.is_empty() {
            let error = format!("record `{}` has no readable fields", declaration.name);
            self.records.insert(record, Err(error.clone()));
            visiting.remove(&record);
            return Err(error);
        }

        let mut offset = 0;
        let mut alignment = 1;
        let mut fields = Vec::with_capacity(declaration.fields.len());
        for field in &declaration.fields {
            let ty = semantics
                .record_field_type(field.id)
                .expect("checked record fields have semantic types");
            let (field_size, field_alignment) = match semantics.types().kind(ty) {
                TypeKind::Builtin(builtin) => scalar_layout(*builtin).ok_or_else(|| {
                    format!(
                        "record `{}.{}` is not MemoryReadable because `{builtin}` has no fixed process-memory layout",
                        declaration.name, field.name
                    )
                })?,
                TypeKind::Record(nested) => {
                    let nested = self
                        .build_record(*nested, records, semantics, visiting)
                        .map_err(|error| {
                            format!(
                                "record `{}.{}` is not MemoryReadable: {error}",
                                declaration.name, field.name
                            )
                        })?;
                    (nested.size, nested.alignment)
                }
                kind => {
                    return Err(format!(
                        "record `{}.{}` is not MemoryReadable because `{kind:?}` has no fixed process-memory layout",
                        declaration.name, field.name
                    ));
                }
            };
            offset = align_up(offset, field_alignment);
            fields.push(MemoryFieldLayout {
                field: field.id,
                ty,
                offset,
            });
            offset = offset
                .checked_add(field_size)
                .ok_or_else(|| format!("record `{}` is too large", declaration.name))?;
            alignment = alignment.max(field_alignment);
        }
        let size = align_up(offset, alignment);
        let layout = RecordMemoryLayout {
            record,
            size,
            alignment,
            fields,
        };
        visiting.remove(&record);
        self.records.insert(record, Ok(layout.clone()));
        Ok(layout)
    }
}

fn scalar_layout(ty: BuiltinType) -> Option<(u32, u32)> {
    let size = match ty {
        BuiltinType::Bool | BuiltinType::I8 | BuiltinType::U8 => 1,
        BuiltinType::I16 | BuiltinType::U16 => 2,
        BuiltinType::I32 | BuiltinType::U32 | BuiltinType::F32 => 4,
        BuiltinType::I64 | BuiltinType::U64 | BuiltinType::Address | BuiltinType::F64 => 8,
        _ => return None,
    };
    Some((size, size))
}

fn align_up(value: u32, alignment: u32) -> u32 {
    debug_assert!(alignment.is_power_of_two());
    value.saturating_add(alignment - 1) & !(alignment - 1)
}
